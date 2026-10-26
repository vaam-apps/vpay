//! The port the merchant MCP tools reach the rails through (ADR-0033 D3).
//!
//! # Why a port, and why here
//!
//! The MCP tools are procedures in `schemas/vpay.cstack`, so their bodies
//! live in this crate, inside `impl procedures::ProcedureRegistry for
//! Payments`. But what a write tool must do — claim the idempotency key,
//! validate against configuration, call a rail — is `vpay-api`'s, and it
//! sits above this crate. This trait is how the procedure body reaches it
//! without this crate depending upward: `vpay-api` implements it, and hands
//! an `Arc<dyn MerchantOperations>` to [`crate::Repositories::merchant_mcp_router`].
//! It is the same shape ADR-0002 gave the rails: the lower layer owns the
//! trait, the upper layer the implementation.
//!
//! # What crosses it, in both directions
//!
//! In: a [`MerchantCall`], whose merchant comes from the authenticated
//! context — never from the agent's arguments — and whose operation is a
//! typed request. This crate does not know `/v1`'s form encoding; the
//! implementation does.
//!
//! Out: an [`OperationOutcome`], whose `body` is **already** in the shape the
//! tool's output type declares. Reshaping `/v1`'s objects is the
//! implementation's job, because `vpay-api` owns those objects; this crate
//! only deserializes. A refusal is an [`OperationRefusal`] carrying `/v1`'s
//! own error `code`, `message` and `param`, so an agent is told exactly what
//! a `/v1` caller would be.

use async_trait::async_trait;

/// One merchant operation, as a tool call asked for it.
///
/// `merchant_id` is the authenticated caller's tenant
/// (`CratestackContext::tenant_id`). There is no other way to set it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerchantCall {
    /// The tenant the call acts as.
    pub merchant_id: String,
    /// What it asks for.
    pub operation: MerchantOperation,
}

/// The operations the merchant MCP surface can perform.
///
/// Every write carries an `idempotency_key`, because every `/v1` write
/// requires one; the implementation claims it in vpay's own per-merchant
/// store (ADR-0033 D7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MerchantOperation {
    /// `GET /v1/payment_intents/{id}`.
    RetrievePaymentIntent {
        /// The intent's id.
        id: String,
    },
    /// `POST /v1/payment_intents`.
    CreatePaymentIntent {
        /// The caller's idempotency key.
        idempotency_key: String,
        /// The request.
        request: NewIntentRequest,
    },
    /// `POST /v1/payment_intents/{id}/confirm`.
    ConfirmPaymentIntent {
        /// The caller's idempotency key.
        idempotency_key: String,
        /// The intent's id.
        id: String,
        /// The request.
        request: ConfirmRequest,
    },
    /// `POST /v1/payment_intents/{id}/cancel`.
    CancelPaymentIntent {
        /// The caller's idempotency key.
        idempotency_key: String,
        /// The intent's id.
        id: String,
    },
    /// `GET /v1/refunds/{id}`.
    RetrieveRefund {
        /// The refund's id.
        id: String,
    },
    /// `POST /v1/refunds`.
    CreateRefund {
        /// The caller's idempotency key.
        idempotency_key: String,
        /// The request.
        request: NewRefundRequest,
    },
    /// `POST /v1/refunds/{id}/cancel`.
    CancelRefund {
        /// The caller's idempotency key.
        idempotency_key: String,
        /// The refund's id.
        id: String,
    },
}

impl MerchantOperation {
    /// The MCP tool name this operation is served as, which is also what the
    /// idempotency fingerprint is taken under (`"tool/<name>"`).
    #[must_use]
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::RetrievePaymentIntent { .. } => "retrieve_payment_intent",
            Self::CreatePaymentIntent { .. } => "create_payment_intent",
            Self::ConfirmPaymentIntent { .. } => "confirm_payment_intent",
            Self::CancelPaymentIntent { .. } => "cancel_payment_intent",
            Self::RetrieveRefund { .. } => "retrieve_refund",
            Self::CreateRefund { .. } => "create_refund",
            Self::CancelRefund { .. } => "cancel_refund",
        }
    }
}

/// `POST /v1/payment_intents`' fields, typed.
///
/// `amount` is the integer an agent sent. `/v1`'s own rules (positive, in
/// minor units, within the currency's bounds) are applied by the
/// implementation exactly as they are to a form body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIntentRequest {
    /// Minor units.
    pub amount: i64,
    /// ISO 4217, any case; `/v1` normalises it.
    pub currency: String,
    /// The rails or method types this intent may be confirmed on; empty
    /// means `/v1`'s default.
    pub payment_method_types: Vec<String>,
    /// Free text.
    pub description: Option<String>,
    /// A `cus_…` of this merchant's.
    pub customer: Option<String>,
    /// Merchant-authored key/value pairs, in the order sent.
    pub metadata: Vec<(String, String)>,
}

/// `POST /v1/payment_intents/{id}/confirm`'s fields, typed.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfirmRequest {
    /// `payment_method_data[type]`: a rail code or a method type.
    pub payment_method_type: String,
    /// `payment_method_data[<type>][<name>]`, one pair per field.
    pub payer_fields: Vec<(String, String)>,
    /// The merchant's `return_url`, for a redirect rail.
    pub return_url: Option<String>,
}

/// Field names only: a payer field's value is a payer's phone number or
/// account (`schemas/privacy-inventory.yaml`'s `payer_msisdn`).
impl std::fmt::Debug for ConfirmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfirmRequest")
            .field("payment_method_type", &self.payment_method_type)
            .field(
                "payer_fields",
                &self
                    .payer_fields
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("return_url", &self.return_url.as_ref().map(|_| "[set]"))
            .finish()
    }
}

/// `POST /v1/refunds`' fields, typed.
#[derive(Clone, PartialEq, Eq)]
pub struct NewRefundRequest {
    /// The `pi_…` being refunded.
    pub payment_intent: String,
    /// Minor units; `None` refunds what remains.
    pub amount: Option<i64>,
    /// Stripe's reason vocabulary.
    pub reason: Option<String>,
    /// `destination[<type>]`, when the refund names one.
    pub destination_type: Option<String>,
    /// `destination[<type>][<name>]`, one pair per field.
    pub destination_fields: Vec<(String, String)>,
    /// Merchant-authored key/value pairs, in the order sent.
    pub metadata: Vec<(String, String)>,
}

/// Field names only, for [`ConfirmRequest`]'s reason: a destination is a
/// payer's number.
impl std::fmt::Debug for NewRefundRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewRefundRequest")
            .field("payment_intent", &self.payment_intent)
            .field("amount", &self.amount)
            .field("reason", &self.reason)
            .field("destination_type", &self.destination_type)
            .field(
                "destination_fields",
                &self
                    .destination_fields
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("metadata", &self.metadata.len())
            .finish()
    }
}

/// What an operation answered when it succeeded.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationOutcome {
    /// The object, already in the shape the tool's output type declares.
    pub body: serde_json::Value,
}

/// What an operation answered when `/v1` would have answered an error: the
/// status `/v1` would have sent, and its envelope's `code`, `message` and
/// `param`, verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRefusal {
    /// The HTTP status `/v1` would have sent.
    pub status: u16,
    /// `error.code`.
    pub code: String,
    /// `error.message`, the public text.
    pub message: String,
    /// `error.param`, when the refusal names one.
    pub param: Option<String>,
}

/// Performs [`MerchantCall`]s. Implemented by `vpay-api`.
#[async_trait]
pub trait MerchantOperations: Send + Sync {
    /// Runs `call` exactly as the matching `/v1` route would, under
    /// `call.merchant_id`.
    ///
    /// # Errors
    ///
    /// [`OperationRefusal`] for every answer `/v1` sends as an error,
    /// including a replayed one.
    async fn perform(&self, call: MerchantCall) -> Result<OperationOutcome, OperationRefusal>;
}
