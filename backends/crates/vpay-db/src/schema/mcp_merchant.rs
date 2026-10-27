//! The bodies of the merchant MCP procedures (ADR-0033 D3): the `retrieve*`,
//! `open*`, `confirm*`, `cancel*` and `issue*` procedures at the end of
//! `schemas/vpay.cstack`.
//!
//! Each body does three things and no more:
//!
//! 1. takes the tenant from the context (`tenant_of`), never from the
//!    arguments;
//! 2. turns the generated arguments into a [`MerchantOperation`] and asks
//!    the [`MerchantOperations`] port, which `vpay-api` implements, to run it
//!    exactly as `/v1` would;
//! 3. deserializes the answer into the generated output type, or turns `/v1`'s
//!    refusal into a [`CratestackError`] that keeps `/v1`'s `code` and
//!    `param` in its message.
//!
//! The policy has already run by the time a body is reached: the
//! `Authorized` witness each one is handed can only be built by the
//! generated `invoke_with_db` after `@allow` passed (cratestack#512).

use std::sync::Arc;

use cratestack::{CratestackContext, CratestackError};

use super::cratestack_schema::{procedures, types};
use super::search_payment_intents::tenant_of;
use crate::merchant_operations::{
    ConfirmRequest, MerchantCall, MerchantOperation, MerchantOperations, NewIntentRequest,
    NewRefundRequest, OperationRefusal,
};

/// Runs `operation` for the context's tenant and deserializes the answer
/// into `T`, the procedure's output type.
///
/// `ops` is `None` on every registry but the merchant MCP endpoint's: the
/// dashboard's procedure router serves the same generated procedures, and
/// a write reached through it must not run whatever its policy said. It
/// answers `Forbidden`, the same refusal the policy gives, so the two
/// cannot be told apart.
pub(super) async fn perform<T: serde::de::DeserializeOwned>(
    ops: Option<&Arc<dyn MerchantOperations>>,
    ctx: &CratestackContext,
    operation: MerchantOperation,
) -> Result<T, CratestackError> {
    let Some(ops) = ops else {
        return Err(CratestackError::Forbidden(
            "procedure policy denied this operation".to_owned(),
        ));
    };
    let merchant_id = tenant_of(ctx)?.to_owned();
    let outcome = ops
        .perform(MerchantCall {
            merchant_id,
            operation,
        })
        .await
        .map_err(refusal_error)?;
    serde_json::from_value(outcome.body).map_err(|error| {
        // The port promised the output type's shape; a body that does not
        // decode is a defect in `vpay-api`'s reshaping, not the caller's.
        CratestackError::Internal(format!(
            "a merchant operation answered a body its tool's output type cannot hold: {error}"
        ))
    })
}

/// `/v1`'s refusal, as the error the MCP transport renders.
///
/// The status picks the variant, so an agent sees the same class of answer
/// `/v1` gives (`400`, `404`, `409`, …). The message keeps `/v1`'s own
/// `code`, and `param` when there is one, in front of its text: they are
/// what an agent (or a person reading its log) needs to fix the call, and
/// CrateStack's envelope has no field for them.
pub(super) fn refusal_error(refusal: OperationRefusal) -> CratestackError {
    let text = match &refusal.param {
        Some(param) => format!("{} ({param}): {}", refusal.code, refusal.message),
        None => format!("{}: {}", refusal.code, refusal.message),
    };
    match refusal.status {
        400 | 422 => CratestackError::BadRequest(text),
        401 | 403 => CratestackError::Forbidden(text),
        404 => CratestackError::NotFound(text),
        409 => CratestackError::Conflict(text),
        429 => CratestackError::TooManyRequests(text),
        503 => CratestackError::Unavailable(text),
        status if status >= 500 => CratestackError::Internal(text),
        _ => CratestackError::BadRequest(text),
    }
}

fn pairs(entries: Vec<types::MetadataEntry>) -> Vec<(String, String)> {
    entries
        .into_iter()
        .map(|entry| (entry.key, entry.value))
        .collect()
}

fn named(values: Vec<types::NamedValue>) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|value| (value.name, value.value))
        .collect()
}

pub(super) fn retrieve_payment_intent(
    args: procedures::retrieve_payment_intent::Args,
) -> MerchantOperation {
    MerchantOperation::RetrievePaymentIntent { id: args.id }
}

pub(super) fn open_payment_intent(
    args: procedures::open_payment_intent::Args,
) -> MerchantOperation {
    let input = args.args;
    MerchantOperation::CreatePaymentIntent {
        idempotency_key: input.idempotency_key,
        request: NewIntentRequest {
            amount: input.amount,
            currency: input.currency,
            payment_method_types: input.payment_method_types,
            description: input.description,
            customer: input.customer,
            metadata: pairs(input.metadata),
        },
    }
}

pub(super) fn confirm_payment_intent(
    args: procedures::confirm_payment_intent::Args,
) -> MerchantOperation {
    let input = args.args;
    MerchantOperation::ConfirmPaymentIntent {
        idempotency_key: input.idempotency_key,
        id: input.id,
        request: ConfirmRequest {
            payment_method_type: input.payment_method_type,
            payer_fields: named(input.payer_fields),
            return_url: input.return_url,
        },
    }
}

pub(super) fn cancel_payment_intent(
    args: procedures::cancel_payment_intent::Args,
) -> MerchantOperation {
    MerchantOperation::CancelPaymentIntent {
        idempotency_key: args.args.idempotency_key,
        id: args.args.id,
    }
}

pub(super) fn retrieve_refund(args: procedures::retrieve_refund::Args) -> MerchantOperation {
    MerchantOperation::RetrieveRefund { id: args.id }
}

pub(super) fn issue_refund(args: procedures::issue_refund::Args) -> MerchantOperation {
    let input = args.args;
    MerchantOperation::CreateRefund {
        idempotency_key: input.idempotency_key,
        request: NewRefundRequest {
            payment_intent: input.payment_intent,
            amount: input.amount,
            reason: input.reason,
            destination_type: input.destination_type,
            destination_fields: named(input.destination_fields),
            metadata: pairs(input.metadata),
        },
    }
}

pub(super) fn cancel_refund(args: procedures::cancel_refund::Args) -> MerchantOperation {
    MerchantOperation::CancelRefund {
        idempotency_key: args.args.idempotency_key,
        id: args.args.id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(status: u16, param: Option<&str>) -> OperationRefusal {
        OperationRefusal {
            status,
            code: "parameter_invalid".to_owned(),
            message: "Amount must be positive.".to_owned(),
            param: param.map(str::to_owned),
        }
    }

    #[test]
    fn a_refusal_keeps_v1s_code_and_param_and_its_status_class() {
        let error = refusal_error(refusal(400, Some("amount")));
        assert!(matches!(&error, CratestackError::BadRequest(text)
            if text == "parameter_invalid (amount): Amount must be positive."));
        assert!(matches!(
            refusal_error(refusal(404, None)),
            CratestackError::NotFound(_)
        ));
        assert!(matches!(
            refusal_error(refusal(409, None)),
            CratestackError::Conflict(_)
        ));
        assert!(matches!(
            refusal_error(refusal(502, None)),
            CratestackError::Internal(_)
        ));
    }

    #[tokio::test]
    async fn a_registry_without_the_port_refuses_like_the_policy_does() {
        let ctx = crate::mcp_transport::merchant_context(&crate::McpCaller {
            merchant_id: "acct_1".to_owned(),
            can_write: true,
        });
        let result: Result<serde_json::Value, _> = perform(
            None,
            &ctx,
            MerchantOperation::RetrieveRefund {
                id: "re_1".to_owned(),
            },
        )
        .await;
        assert!(matches!(result, Err(CratestackError::Forbidden(_))));
    }
}
