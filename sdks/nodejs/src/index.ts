/**
 * `@vaam-apps/vpay-sdk` — the Node.js merchant SDK for vpay.
 *
 * Every `/v1` call is sealed as a COSE_Sign1 message under the merchant's
 * Ed25519 key and every answer is opened under the pinned server keys — see
 * README.md for the configuration, a usage example, and the errors a refusal
 * becomes. The sealing is `sdks/rust/wasm`, compiled to WebAssembly.
 */
export { VpayClient, type VpayClientOptions } from "./client.js";

export {
  type ExternalSigner,
  type KeyBytes,
  type MerchantAuthOptions,
  type ServerKeysOption,
} from "./sealer.js";

export { verifyWebhook, type VerifyWebhookOptions } from "./webhooks.js";

/** This package's own version, as sent in the `vpay-sdk-node/<version>` User-Agent. */
export { SDK_VERSION } from "./version.js";

export {
  VpayError,
  VpayApiError,
  VpayUnsignedResponseError,
  VpayUnverifiedResponseError,
  VpayUnexpectedResponseError,
  VpayTransportError,
  VpayResponseTooLargeError,
  VpayConfigError,
  VpayInvalidParamsError,
  WebhookSignatureError,
} from "./errors.js";

export type {
  AccountHolder,
  RetrieveAccountHolderParams,
  CheckoutSession,
  CheckoutSessionStatus,
  CheckoutSessionUiMode,
  CheckoutPaymentStatus,
  CreateCheckoutSessionParams,
  ListCheckoutSessionsParams,
  Customer,
  DeletedCustomer,
  Address,
  AddressParams,
  CreateCustomerParams,
  UpdateCustomerParams,
  ListCustomersParams,
  Invoice,
  InvoiceLine,
  InvoiceStatus,
  InvoiceStatusTransitions,
  DeletedInvoice,
  DeletedInvoiceItem,
  CreateInvoiceParams,
  UpdateInvoiceParams,
  ListInvoicesParams,
  PayInvoiceParams,
  HostedPayInvoiceParams,
  OutOfBandPayInvoiceParams,
  OutOfBandParams,
  OutOfBandMethod,
  OutOfBandPayment,
  CreateInvoiceItemParams,
  UpdateInvoiceItemParams,
  PaymentIntent,
  PaymentIntentStatus,
  PaymentMethodDetails,
  PaymentMethodType,
  NextAction,
  LastPaymentError,
  FailureCode,
  Refund,
  RefundStatus,
  Event,
  KnownEventType,
  Balance,
  BalanceAmount,
  List,
  ListParams,
  ListPaymentIntentsParams,
  ListEventsParams,
  CreatePaymentIntentParams,
  ConfirmPaymentIntentParams,
  MtnMomoPaymentMethodData,
  OrangeMoneyPaymentMethodData,
  CreateRefundParams,
  RefundDestination,
  UpdateRefundParams,
  ListRefundsParams,
  RequestOptions,
} from "./types.js";

export {
  isCheckoutSessionEvent,
  isInvoiceEvent,
  isPaymentIntentEvent,
  isRefundEvent,
} from "./types.js";
