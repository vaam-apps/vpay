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
    let (client_id, scopes) = credential_of(ctx)?;
    let outcome = ops
        .perform(MerchantCall {
            merchant_id,
            client_id,
            scopes,
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

/// The authenticated credential a merchant context carries: its OAuth
/// client and its effective scopes (`crate::mcp_transport`'s table).
///
/// A context with no `client_id` claim is refused as the policy refuses:
/// only `merchant_context` builds a context that reaches a body with the
/// port present, and it always sets one, so a miss is a wiring defect that
/// must not run as nobody in particular. A missing or malformed `scopes`
/// claim is the empty list, which can only refuse more, never admit more.
fn credential_of(ctx: &CratestackContext) -> Result<(String, Vec<String>), CratestackError> {
    let client_id = match ctx.auth_field(crate::mcp_transport::CLIENT_ID_CLAIM) {
        Some(cratestack::Value::String(id)) if !id.is_empty() => id.clone(),
        _ => {
            return Err(CratestackError::Forbidden(
                "procedure policy denied this operation".to_owned(),
            ));
        }
    };
    let scopes = match ctx.auth_field(crate::mcp_transport::SCOPES_CLAIM) {
        Some(cratestack::Value::List(values)) => values
            .iter()
            .filter_map(|value| match value {
                cratestack::Value::String(scope) => Some(scope.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok((client_id, scopes))
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
            client_id: "client-1".to_owned(),
            scopes: vec!["payments:write".to_owned()],
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

    /// The port is handed the credential that authenticated, with exactly
    /// its scopes (PR #396 review, P1). Before this, `vpay-api` invented a
    /// principal holding only the payments scopes, so a token that held
    /// `account_holders:read` was refused a nominated-payee refund it was
    /// entitled to, and the lookup budget was kept per tenant, not per key.
    ///
    /// The decisive mutations: drop `client_id`/`scopes` from the call, or
    /// fill them with anything but the context's claims.
    #[tokio::test]
    async fn the_call_carries_the_contexts_client_and_exactly_its_scopes() {
        #[derive(Default)]
        struct Capturing(std::sync::Mutex<Vec<MerchantCall>>);

        #[async_trait::async_trait]
        impl MerchantOperations for Capturing {
            async fn perform(
                &self,
                call: MerchantCall,
            ) -> Result<crate::merchant_operations::OperationOutcome, OperationRefusal>
            {
                if let Ok(mut calls) = self.0.lock() {
                    calls.push(call);
                }
                Ok(crate::merchant_operations::OperationOutcome {
                    body: serde_json::json!({}),
                })
            }
        }

        let capturing = Arc::new(Capturing::default());
        let ops: Arc<dyn MerchantOperations> = capturing.clone();
        let ctx = crate::mcp_transport::merchant_context(&crate::McpCaller {
            merchant_id: "acct_1".to_owned(),
            can_write: true,
            client_id: "client-1".to_owned(),
            scopes: vec![
                "payments:write".to_owned(),
                "account_holders:read".to_owned(),
            ],
        });
        let _: serde_json::Value = perform(
            Some(&ops),
            &ctx,
            MerchantOperation::RetrieveRefund {
                id: "re_1".to_owned(),
            },
        )
        .await
        .expect("the capturing port answers");

        let calls = capturing.0.lock().expect("not poisoned").clone();
        let call = calls.first().expect("one call reached the port");
        assert_eq!(call.merchant_id, "acct_1");
        assert_eq!(call.client_id, "client-1");
        assert_eq!(
            call.scopes,
            vec![
                "payments:write".to_owned(),
                "account_holders:read".to_owned()
            ]
        );
    }

    /// A context with no client reaches no port: it is a wiring defect, and
    /// running it as nobody in particular is how a per-key budget is bypassed.
    #[test]
    fn a_context_without_a_client_is_refused_before_the_port() {
        let ctx = crate::mcp_transport::merchant_context(&crate::McpCaller {
            merchant_id: "acct_1".to_owned(),
            can_write: true,
            client_id: String::new(),
            scopes: Vec::new(),
        });
        assert!(matches!(
            credential_of(&ctx),
            Err(CratestackError::Forbidden(_))
        ));
    }
}
