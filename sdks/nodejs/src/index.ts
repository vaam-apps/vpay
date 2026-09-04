/**
 * `@vpay/sdk` — the Node.js merchant SDK for vpay.
 *
 * See README.md for the handshake, a usage example, and this package's own
 * Status section (the server serves `/v1/oauth` and the `/v1` auth boundary;
 * no `/v1` resource route exists yet).
 */
export { VpayClient, type VpayClientOptions } from "./client.js";

export {
  mintClientAssertion,
  type MintClientAssertionOptions,
} from "./auth.js";

export { verifyWebhook, type VerifyWebhookOptions } from "./webhooks.js";

/** This package's own version, as sent in the `vpay-sdk-node/<version>` User-Agent. */
export { SDK_VERSION } from "./version.js";

export {
  VpayError,
  VpayApiError,
  VpayAuthError,
  VpayUnexpectedResponseError,
  VpayTransportError,
  VpayConfigError,
  WebhookSignatureError,
} from "./errors.js";

export type {
  CheckoutSession,
  CheckoutSessionStatus,
  CheckoutSessionUiMode,
  CheckoutPaymentStatus,
  CreateCheckoutSessionParams,
  ListCheckoutSessionsParams,
  PaymentIntent,
  PaymentIntentStatus,
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
  ListEventsParams,
  CreatePaymentIntentParams,
  ConfirmPaymentIntentParams,
  MtnMomoPaymentMethodData,
  OrangeMoneyPaymentMethodData,
  CreateRefundParams,
  RequestOptions,
} from "./types.js";

export { isPaymentIntentEvent, isRefundEvent } from "./types.js";
