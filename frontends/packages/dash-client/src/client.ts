import { CratestackRuntime, type CratestackClientOptions, type CratestackResponseEnvelope } from "./runtime.js";
import { reviveWireFields, revivePagedWireFields, reviveWireScalar } from "./models.js";
import type {
  PaymentIntentSummary,
  PaymentIntentListFilter,
  RefundSummary,
  RefundListFilter,
  WebhookDeliverySummary,
  WebhookDeliveryListFilter,
  CustomerSummary,
  CustomerListFilter,
  CheckoutSessionSummary,
  CheckoutSessionListFilter,
  MetadataEntry,
  NamedValue,
  McpRedirect,
  McpNextAction,
  McpLastPaymentError,
  McpPaymentMethodDetails,
  McpPaymentIntent,
  McpRefund,
  OpenPaymentIntentInput,
  ConfirmPaymentIntentInput,
  CancelInput,
  IssueRefundInput,
  Currency,
  CreateCurrencyInput,
  UpdateCurrencyInput,
  CurrencyWhere,
  CurrencyOrderByClause,
  CurrencyFindMany,
  Provider,
  CreateProviderInput,
  UpdateProviderInput,
  ProviderWhere,
  ProviderOrderByClause,
  ProviderFindMany,
  PaymentIntent,
  UpdatePaymentIntentInput,
  PaymentIntentWhere,
  PaymentIntentOrderByClause,
  PaymentIntentFindMany,
  Charge,
  UpdateChargeInput,
  ChargeWhere,
  ChargeOrderByClause,
  ChargeFindMany,
  Refund,
  UpdateRefundInput,
  RefundWhere,
  RefundOrderByClause,
  RefundFindMany,
  CheckoutSession,
  UpdateCheckoutSessionInput,
  CheckoutSessionWhere,
  CheckoutSessionOrderByClause,
  CheckoutSessionFindMany,
  LedgerTransaction,
  UpdateLedgerTransactionInput,
  LedgerTransactionWhere,
  LedgerTransactionOrderByClause,
  LedgerTransactionFindMany,
  LedgerEntry,
  UpdateLedgerEntryInput,
  LedgerEntryWhere,
  LedgerEntryOrderByClause,
  LedgerEntryFindMany,
  DisabledClient,
  CreateDisabledClientInput,
  UpdateDisabledClientInput,
  DisabledClientWhere,
  DisabledClientOrderByClause,
  DisabledClientFindMany,
  Event,
  UpdateEventInput,
  EventWhere,
  EventOrderByClause,
  EventFindMany,
  WebhookDelivery,
  CreateWebhookDeliveryInput,
  UpdateWebhookDeliveryInput,
  WebhookDeliveryWhere,
  WebhookDeliveryOrderByClause,
  WebhookDeliveryFindMany,
  Customer,
  UpdateCustomerInput,
  CustomerWhere,
  CustomerOrderByClause,
  CustomerFindMany,
  StaffMember,
  CreateStaffMemberInput,
  UpdateStaffMemberInput,
  StaffMemberWhere,
  StaffMemberOrderByClause,
  StaffMemberFindMany,
  Credential,
  CreateCredentialInput,
  UpdateCredentialInput,
  CredentialWhere,
  CredentialOrderByClause,
  CredentialFindMany,
  Invoice,
  UpdateInvoiceInput,
  InvoiceWhere,
  InvoiceOrderByClause,
  InvoiceFindMany,
  InvoiceItem,
  UpdateInvoiceItemInput,
  InvoiceItemWhere,
  InvoiceItemOrderByClause,
  InvoiceItemFindMany,
  ManualPayment,
  UpdateManualPaymentInput,
  ManualPaymentWhere,
  ManualPaymentOrderByClause,
  ManualPaymentFindMany,
  RateLimitWindow,
  UpdateRateLimitWindowInput,
  RateLimitWindowWhere,
  RateLimitWindowOrderByClause,
  RateLimitWindowFindMany,
  EnvelopeNonce,
  UpdateEnvelopeNonceInput,
  EnvelopeNonceWhere,
  EnvelopeNonceOrderByClause,
  EnvelopeNonceFindMany,
  ConsoleKey,
  CreateConsoleKeyInput,
  UpdateConsoleKeyInput,
  ConsoleKeyWhere,
  ConsoleKeyOrderByClause,
  ConsoleKeyFindMany,
  SearchPaymentIntentsArgs,
  SearchRefundsArgs,
  SearchWebhookDeliveriesArgs,
  SearchCustomersArgs,
  SearchCheckoutSessionsArgs,
  RetrievePaymentIntentArgs,
  OpenPaymentIntentArgs,
  ConfirmPaymentIntentArgs,
  CancelPaymentIntentArgs,
  RetrieveRefundArgs,
  IssueRefundArgs,
  CancelRefundArgs,
  ProviderFlow,
  IntentStatus,
  ChargeState,
  FailureCode,
  RefundStatus,
  AccountKind,
  Direction,
  InvoiceStatus,
  ManualPaymentMethod,
  CurrencySortField,
  ProviderSortField,
  PaymentIntentSortField,
  ChargeSortField,
  RefundSortField,
  CheckoutSessionSortField,
  LedgerTransactionSortField,
  LedgerEntrySortField,
  DisabledClientSortField,
  EventSortField,
  WebhookDeliverySortField,
  CustomerSortField,
  StaffMemberSortField,
  CredentialSortField,
  InvoiceSortField,
  InvoiceItemSortField,
  ManualPaymentSortField,
  RateLimitWindowSortField,
  EnvelopeNonceSortField,
  ConsoleKeySortField,
  Page,
} from "./models.js";
import {
  toSearchQuery,
  withIfMatchHeader,
  type CratestackQueryRequestConfig,
  type CratestackRequestConfig,
  type CratestackWriteRequestConfig,
} from "./queries.js";

export class VpayDashClientClient {
  readonly runtime: CratestackRuntime;
  readonly procedures: ProceduresApi;
  readonly currencies: CurrencyApi;
  readonly providers: ProviderApi;
  readonly paymentIntents: PaymentIntentApi;
  readonly charges: ChargeApi;
  readonly refunds: RefundApi;
  readonly checkoutSessions: CheckoutSessionApi;
  readonly ledgerTransactions: LedgerTransactionApi;
  readonly ledgerEntries: LedgerEntryApi;
  readonly disabledClients: DisabledClientApi;
  readonly events: EventApi;
  readonly webhookDeliveries: WebhookDeliveryApi;
  readonly customers: CustomerApi;
  readonly staffMembers: StaffMemberApi;
  readonly credentials: CredentialApi;
  readonly invoices: InvoiceApi;
  readonly invoiceItems: InvoiceItemApi;
  readonly manualPayments: ManualPaymentApi;
  readonly rateLimitWindows: RateLimitWindowApi;
  readonly envelopeNonces: EnvelopeNonceApi;
  readonly consoleKeys: ConsoleKeyApi;

  constructor(originOrRuntime: string | CratestackRuntime, options: CratestackClientOptions = {}) {
    this.runtime = typeof originOrRuntime === "string"
      ? new CratestackRuntime(originOrRuntime, options)
      : originOrRuntime;
    this.procedures = new ProceduresApi(this.runtime);
    this.currencies = new CurrencyApi(this.runtime);
    this.providers = new ProviderApi(this.runtime);
    this.paymentIntents = new PaymentIntentApi(this.runtime);
    this.charges = new ChargeApi(this.runtime);
    this.refunds = new RefundApi(this.runtime);
    this.checkoutSessions = new CheckoutSessionApi(this.runtime);
    this.ledgerTransactions = new LedgerTransactionApi(this.runtime);
    this.ledgerEntries = new LedgerEntryApi(this.runtime);
    this.disabledClients = new DisabledClientApi(this.runtime);
    this.events = new EventApi(this.runtime);
    this.webhookDeliveries = new WebhookDeliveryApi(this.runtime);
    this.customers = new CustomerApi(this.runtime);
    this.staffMembers = new StaffMemberApi(this.runtime);
    this.credentials = new CredentialApi(this.runtime);
    this.invoices = new InvoiceApi(this.runtime);
    this.invoiceItems = new InvoiceItemApi(this.runtime);
    this.manualPayments = new ManualPaymentApi(this.runtime);
    this.rateLimitWindows = new RateLimitWindowApi(this.runtime);
    this.envelopeNonces = new EnvelopeNonceApi(this.runtime);
    this.consoleKeys = new ConsoleKeyApi(this.runtime);
  }
}

export class CurrencyApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Currency[]> {
    return this.runtime.get<unknown>("/currencies", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Currency') as Currency[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Currency> {
    return this.runtime.get<unknown>(`/currencies/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Currency') as Currency);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Currency>> {
    return this.runtime.getWithResponse<unknown>(`/currencies/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Currency') as Currency,
      response: result.response,
    }));
  }

  create(input: CreateCurrencyInput, options: CratestackRequestConfig = {}): Promise<Currency> {
    return this.runtime.post<unknown>("/currencies", input, options)
      .then((value) => reviveWireFields(value, 'Currency') as Currency);
  }

  update(
    id: string,
    input: UpdateCurrencyInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Currency> {
    return this.runtime.patch<unknown>(`/currencies/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Currency') as Currency);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/currencies/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ProviderApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Provider[]> {
    return this.runtime.get<unknown>("/providers", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Provider') as Provider[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Provider> {
    return this.runtime.get<unknown>(`/providers/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Provider') as Provider);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Provider>> {
    return this.runtime.getWithResponse<unknown>(`/providers/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Provider') as Provider,
      response: result.response,
    }));
  }

  create(input: CreateProviderInput, options: CratestackRequestConfig = {}): Promise<Provider> {
    return this.runtime.post<unknown>("/providers", input, options)
      .then((value) => reviveWireFields(value, 'Provider') as Provider);
  }

  update(
    id: string,
    input: UpdateProviderInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Provider> {
    return this.runtime.patch<unknown>(`/providers/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Provider') as Provider);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/providers/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class PaymentIntentApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<PaymentIntent[]> {
    return this.runtime.get<unknown>("/payment_intents", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'PaymentIntent') as PaymentIntent[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<PaymentIntent> {
    return this.runtime.get<unknown>(`/payment_intents/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'PaymentIntent') as PaymentIntent);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<PaymentIntent>> {
    return this.runtime.getWithResponse<unknown>(`/payment_intents/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'PaymentIntent') as PaymentIntent,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdatePaymentIntentInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<PaymentIntent> {
    return this.runtime.patch<unknown>(`/payment_intents/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'PaymentIntent') as PaymentIntent);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/payment_intents/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ChargeApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Charge[]> {
    return this.runtime.get<unknown>("/charges", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Charge') as Charge[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Charge> {
    return this.runtime.get<unknown>(`/charges/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Charge') as Charge);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Charge>> {
    return this.runtime.getWithResponse<unknown>(`/charges/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Charge') as Charge,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateChargeInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Charge> {
    return this.runtime.patch<unknown>(`/charges/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Charge') as Charge);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/charges/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class RefundApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Refund[]> {
    return this.runtime.get<unknown>("/refunds", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Refund') as Refund[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Refund> {
    return this.runtime.get<unknown>(`/refunds/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Refund') as Refund);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Refund>> {
    return this.runtime.getWithResponse<unknown>(`/refunds/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Refund') as Refund,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateRefundInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Refund> {
    return this.runtime.patch<unknown>(`/refunds/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Refund') as Refund);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/refunds/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class CheckoutSessionApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<CheckoutSession[]> {
    return this.runtime.get<unknown>("/checkout_sessions", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'CheckoutSession') as CheckoutSession[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<CheckoutSession> {
    return this.runtime.get<unknown>(`/checkout_sessions/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'CheckoutSession') as CheckoutSession);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<CheckoutSession>> {
    return this.runtime.getWithResponse<unknown>(`/checkout_sessions/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'CheckoutSession') as CheckoutSession,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateCheckoutSessionInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<CheckoutSession> {
    return this.runtime.patch<unknown>(`/checkout_sessions/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'CheckoutSession') as CheckoutSession);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/checkout_sessions/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class LedgerTransactionApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<LedgerTransaction[]> {
    return this.runtime.get<unknown>("/ledger_transactions", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'LedgerTransaction') as LedgerTransaction[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<LedgerTransaction> {
    return this.runtime.get<unknown>(`/ledger_transactions/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'LedgerTransaction') as LedgerTransaction);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<LedgerTransaction>> {
    return this.runtime.getWithResponse<unknown>(`/ledger_transactions/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'LedgerTransaction') as LedgerTransaction,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateLedgerTransactionInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<LedgerTransaction> {
    return this.runtime.patch<unknown>(`/ledger_transactions/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'LedgerTransaction') as LedgerTransaction);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/ledger_transactions/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class LedgerEntryApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<LedgerEntry[]> {
    return this.runtime.get<unknown>("/ledger_entries", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'LedgerEntry') as LedgerEntry[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<LedgerEntry> {
    return this.runtime.get<unknown>(`/ledger_entries/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'LedgerEntry') as LedgerEntry);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<LedgerEntry>> {
    return this.runtime.getWithResponse<unknown>(`/ledger_entries/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'LedgerEntry') as LedgerEntry,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateLedgerEntryInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<LedgerEntry> {
    return this.runtime.patch<unknown>(`/ledger_entries/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'LedgerEntry') as LedgerEntry);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/ledger_entries/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class DisabledClientApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<DisabledClient[]> {
    return this.runtime.get<unknown>("/disabled_clients", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'DisabledClient') as DisabledClient[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<DisabledClient> {
    return this.runtime.get<unknown>(`/disabled_clients/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'DisabledClient') as DisabledClient);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<DisabledClient>> {
    return this.runtime.getWithResponse<unknown>(`/disabled_clients/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'DisabledClient') as DisabledClient,
      response: result.response,
    }));
  }

  create(input: CreateDisabledClientInput, options: CratestackRequestConfig = {}): Promise<DisabledClient> {
    return this.runtime.post<unknown>("/disabled_clients", input, options)
      .then((value) => reviveWireFields(value, 'DisabledClient') as DisabledClient);
  }

  update(
    id: string,
    input: UpdateDisabledClientInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<DisabledClient> {
    return this.runtime.patch<unknown>(`/disabled_clients/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'DisabledClient') as DisabledClient);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/disabled_clients/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class EventApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Event[]> {
    return this.runtime.get<unknown>("/events", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Event') as Event[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Event> {
    return this.runtime.get<unknown>(`/events/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Event') as Event);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Event>> {
    return this.runtime.getWithResponse<unknown>(`/events/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Event') as Event,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateEventInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Event> {
    return this.runtime.patch<unknown>(`/events/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Event') as Event);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/events/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class WebhookDeliveryApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<WebhookDelivery[]> {
    return this.runtime.get<unknown>("/webhook_deliveries", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'WebhookDelivery') as WebhookDelivery[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<WebhookDelivery> {
    return this.runtime.get<unknown>(`/webhook_deliveries/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'WebhookDelivery') as WebhookDelivery);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<WebhookDelivery>> {
    return this.runtime.getWithResponse<unknown>(`/webhook_deliveries/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'WebhookDelivery') as WebhookDelivery,
      response: result.response,
    }));
  }

  create(input: CreateWebhookDeliveryInput, options: CratestackRequestConfig = {}): Promise<WebhookDelivery> {
    return this.runtime.post<unknown>("/webhook_deliveries", input, options)
      .then((value) => reviveWireFields(value, 'WebhookDelivery') as WebhookDelivery);
  }

  update(
    id: string,
    input: UpdateWebhookDeliveryInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<WebhookDelivery> {
    return this.runtime.patch<unknown>(`/webhook_deliveries/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'WebhookDelivery') as WebhookDelivery);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/webhook_deliveries/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class CustomerApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Customer[]> {
    return this.runtime.get<unknown>("/customers", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Customer') as Customer[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Customer> {
    return this.runtime.get<unknown>(`/customers/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Customer') as Customer);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Customer>> {
    return this.runtime.getWithResponse<unknown>(`/customers/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Customer') as Customer,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateCustomerInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Customer> {
    return this.runtime.patch<unknown>(`/customers/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Customer') as Customer);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/customers/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class StaffMemberApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<StaffMember[]> {
    return this.runtime.get<unknown>("/staff_members", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'StaffMember') as StaffMember[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<StaffMember> {
    return this.runtime.get<unknown>(`/staff_members/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'StaffMember') as StaffMember);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<StaffMember>> {
    return this.runtime.getWithResponse<unknown>(`/staff_members/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'StaffMember') as StaffMember,
      response: result.response,
    }));
  }

  create(input: CreateStaffMemberInput, options: CratestackRequestConfig = {}): Promise<StaffMember> {
    return this.runtime.post<unknown>("/staff_members", input, options)
      .then((value) => reviveWireFields(value, 'StaffMember') as StaffMember);
  }

  update(
    id: string,
    input: UpdateStaffMemberInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<StaffMember> {
    return this.runtime.patch<unknown>(`/staff_members/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'StaffMember') as StaffMember);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/staff_members/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class CredentialApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Credential[]> {
    return this.runtime.get<unknown>("/credentials", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Credential') as Credential[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Credential> {
    return this.runtime.get<unknown>(`/credentials/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Credential') as Credential);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Credential>> {
    return this.runtime.getWithResponse<unknown>(`/credentials/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Credential') as Credential,
      response: result.response,
    }));
  }

  create(input: CreateCredentialInput, options: CratestackRequestConfig = {}): Promise<Credential> {
    return this.runtime.post<unknown>("/credentials", input, options)
      .then((value) => reviveWireFields(value, 'Credential') as Credential);
  }

  update(
    id: string,
    input: UpdateCredentialInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Credential> {
    return this.runtime.patch<unknown>(`/credentials/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Credential') as Credential);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/credentials/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class InvoiceApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<Invoice[]> {
    return this.runtime.get<unknown>("/invoices", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Invoice') as Invoice[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<Invoice> {
    return this.runtime.get<unknown>(`/invoices/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'Invoice') as Invoice);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<Invoice>> {
    return this.runtime.getWithResponse<unknown>(`/invoices/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'Invoice') as Invoice,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateInvoiceInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<Invoice> {
    return this.runtime.patch<unknown>(`/invoices/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'Invoice') as Invoice);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/invoices/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class InvoiceItemApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<InvoiceItem[]> {
    return this.runtime.get<unknown>("/invoice_items", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'InvoiceItem') as InvoiceItem[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<InvoiceItem> {
    return this.runtime.get<unknown>(`/invoice_items/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'InvoiceItem') as InvoiceItem);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<InvoiceItem>> {
    return this.runtime.getWithResponse<unknown>(`/invoice_items/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'InvoiceItem') as InvoiceItem,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateInvoiceItemInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<InvoiceItem> {
    return this.runtime.patch<unknown>(`/invoice_items/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'InvoiceItem') as InvoiceItem);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/invoice_items/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ManualPaymentApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<ManualPayment[]> {
    return this.runtime.get<unknown>("/manual_payments", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'ManualPayment') as ManualPayment[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<ManualPayment> {
    return this.runtime.get<unknown>(`/manual_payments/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'ManualPayment') as ManualPayment);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<ManualPayment>> {
    return this.runtime.getWithResponse<unknown>(`/manual_payments/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'ManualPayment') as ManualPayment,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateManualPaymentInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<ManualPayment> {
    return this.runtime.patch<unknown>(`/manual_payments/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'ManualPayment') as ManualPayment);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/manual_payments/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class RateLimitWindowApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<RateLimitWindow[]> {
    return this.runtime.get<unknown>("/rate_limit_windows", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'RateLimitWindow') as RateLimitWindow[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<RateLimitWindow> {
    return this.runtime.get<unknown>(`/rate_limit_windows/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'RateLimitWindow') as RateLimitWindow);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<RateLimitWindow>> {
    return this.runtime.getWithResponse<unknown>(`/rate_limit_windows/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'RateLimitWindow') as RateLimitWindow,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateRateLimitWindowInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<RateLimitWindow> {
    return this.runtime.patch<unknown>(`/rate_limit_windows/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'RateLimitWindow') as RateLimitWindow);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/rate_limit_windows/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class EnvelopeNonceApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<EnvelopeNonce[]> {
    return this.runtime.get<unknown>("/envelope_nonces", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'EnvelopeNonce') as EnvelopeNonce[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<EnvelopeNonce> {
    return this.runtime.get<unknown>(`/envelope_nonces/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'EnvelopeNonce') as EnvelopeNonce);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<EnvelopeNonce>> {
    return this.runtime.getWithResponse<unknown>(`/envelope_nonces/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'EnvelopeNonce') as EnvelopeNonce,
      response: result.response,
    }));
  }

  update(
    id: string,
    input: UpdateEnvelopeNonceInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<EnvelopeNonce> {
    return this.runtime.patch<unknown>(`/envelope_nonces/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'EnvelopeNonce') as EnvelopeNonce);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/envelope_nonces/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ConsoleKeyApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  list(options: CratestackQueryRequestConfig = {}): Promise<ConsoleKey[]> {
    return this.runtime.get<unknown>("/console_keys", {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'ConsoleKey') as ConsoleKey[]);
  }

  get(id: string, options: CratestackQueryRequestConfig = {}): Promise<ConsoleKey> {
    return this.runtime.get<unknown>(`/console_keys/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((value) => reviveWireFields(value, 'ConsoleKey') as ConsoleKey);
  }

  // Same call as `get`, but returns the response alongside the record
  // (issue #610) — read `.response.headers.get("etag")` off the result
  // to get the value `update`/`delete`'s `ifMatch` option needs.
  getWithResponse(
    id: string,
    options: CratestackQueryRequestConfig = {},
  ): Promise<CratestackResponseEnvelope<ConsoleKey>> {
    return this.runtime.getWithResponse<unknown>(`/console_keys/${encodeURIComponent(String(id))}`, {
      headers: options.headers,
      query: toSearchQuery(options.query),
      signal: options.signal,
    }).then((result) => ({
      value: reviveWireFields(result.value, 'ConsoleKey') as ConsoleKey,
      response: result.response,
    }));
  }

  create(input: CreateConsoleKeyInput, options: CratestackRequestConfig = {}): Promise<ConsoleKey> {
    return this.runtime.post<unknown>("/console_keys", input, options)
      .then((value) => reviveWireFields(value, 'ConsoleKey') as ConsoleKey);
  }

  update(
    id: string,
    input: UpdateConsoleKeyInput,
    options: CratestackWriteRequestConfig = {},
  ): Promise<ConsoleKey> {
    return this.runtime.patch<unknown>(`/console_keys/${encodeURIComponent(String(id))}`, input, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    })
      .then((value) => reviveWireFields(value, 'ConsoleKey') as ConsoleKey);
  }

  delete(id: string, options: CratestackWriteRequestConfig = {}): Promise<void> {
    return this.runtime.delete<void>(`/console_keys/${encodeURIComponent(String(id))}`, {
      headers: withIfMatchHeader(options.headers, options.ifMatch),
      signal: options.signal,
    });
  }
}

export class ProceduresApi {
  constructor(private readonly runtime: CratestackRuntime) {}

  searchPaymentIntents(args: SearchPaymentIntentsArgs, options: CratestackRequestConfig = {}): Promise<Page<PaymentIntentSummary>> {
    return this.runtime.post<unknown>("/$procs/searchPaymentIntents", args, options)
      .then((value) => revivePagedWireFields(value, 'PaymentIntentSummary') as Page<PaymentIntentSummary>);
  }

  searchRefunds(args: SearchRefundsArgs, options: CratestackRequestConfig = {}): Promise<Page<RefundSummary>> {
    return this.runtime.post<unknown>("/$procs/searchRefunds", args, options)
      .then((value) => revivePagedWireFields(value, 'RefundSummary') as Page<RefundSummary>);
  }

  searchWebhookDeliveries(args: SearchWebhookDeliveriesArgs, options: CratestackRequestConfig = {}): Promise<Page<WebhookDeliverySummary>> {
    return this.runtime.post<unknown>("/$procs/searchWebhookDeliveries", args, options)
      .then((value) => revivePagedWireFields(value, 'WebhookDeliverySummary') as Page<WebhookDeliverySummary>);
  }

  searchCustomers(args: SearchCustomersArgs, options: CratestackRequestConfig = {}): Promise<Page<CustomerSummary>> {
    return this.runtime.post<unknown>("/$procs/searchCustomers", args, options)
      .then((value) => revivePagedWireFields(value, 'CustomerSummary') as Page<CustomerSummary>);
  }

  searchCheckoutSessions(args: SearchCheckoutSessionsArgs, options: CratestackRequestConfig = {}): Promise<Page<CheckoutSessionSummary>> {
    return this.runtime.post<unknown>("/$procs/searchCheckoutSessions", args, options)
      .then((value) => revivePagedWireFields(value, 'CheckoutSessionSummary') as Page<CheckoutSessionSummary>);
  }

  retrievePaymentIntent(args: RetrievePaymentIntentArgs, options: CratestackRequestConfig = {}): Promise<McpPaymentIntent> {
    return this.runtime.post<unknown>("/$procs/retrievePaymentIntent", args, options)
      .then((value) => reviveWireFields(value, 'McpPaymentIntent') as McpPaymentIntent);
  }

  openPaymentIntent(args: OpenPaymentIntentArgs, options: CratestackRequestConfig = {}): Promise<McpPaymentIntent> {
    return this.runtime.post<unknown>("/$procs/openPaymentIntent", args, options)
      .then((value) => reviveWireFields(value, 'McpPaymentIntent') as McpPaymentIntent);
  }

  confirmPaymentIntent(args: ConfirmPaymentIntentArgs, options: CratestackRequestConfig = {}): Promise<McpPaymentIntent> {
    return this.runtime.post<unknown>("/$procs/confirmPaymentIntent", args, options)
      .then((value) => reviveWireFields(value, 'McpPaymentIntent') as McpPaymentIntent);
  }

  cancelPaymentIntent(args: CancelPaymentIntentArgs, options: CratestackRequestConfig = {}): Promise<McpPaymentIntent> {
    return this.runtime.post<unknown>("/$procs/cancelPaymentIntent", args, options)
      .then((value) => reviveWireFields(value, 'McpPaymentIntent') as McpPaymentIntent);
  }

  retrieveRefund(args: RetrieveRefundArgs, options: CratestackRequestConfig = {}): Promise<McpRefund> {
    return this.runtime.post<unknown>("/$procs/retrieveRefund", args, options)
      .then((value) => reviveWireFields(value, 'McpRefund') as McpRefund);
  }

  issueRefund(args: IssueRefundArgs, options: CratestackRequestConfig = {}): Promise<McpRefund> {
    return this.runtime.post<unknown>("/$procs/issueRefund", args, options)
      .then((value) => reviveWireFields(value, 'McpRefund') as McpRefund);
  }

  cancelRefund(args: CancelRefundArgs, options: CratestackRequestConfig = {}): Promise<McpRefund> {
    return this.runtime.post<unknown>("/$procs/cancelRefund", args, options)
      .then((value) => reviveWireFields(value, 'McpRefund') as McpRefund);
  }

}