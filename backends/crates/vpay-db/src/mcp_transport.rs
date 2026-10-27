//! The merchant MCP endpoint's auth seam (ADR-0033 D9).
//!
//! `vpay-api` authenticates the bearer token: it owns the JWT validator, the
//! audience and the client-to-merchant map. What it hands back is an
//! [`McpCaller`], and this module turns that into the `CratestackContext`
//! the generated procedures' policies read. `vpay-api` therefore never
//! builds a CrateStack context and never names a CrateStack type, which is
//! [`crate::DashboardAuthFn`]'s arrangement for the dashboard, for the same
//! reason.
//!
//! # The context, claim by claim
//!
//! | Claim | Value | Read by |
//! | --- | --- | --- |
//! | `tenant.id` | the merchant | every procedure body, through `tenant_of` |
//! | `audience` | `"merchant"` | every MCP procedure's `@allow` |
//! | `can_write` | the token holds `payments:write` | every write's `@allow` |
//!
//! Never a system context: nothing here calls
//! [`crate::persistence`]'s `system_context`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use cratestack::{CratestackContext, CratestackError, PrincipalContext, PrincipalFacet, Value};

/// The `audience` claim a merchant MCP context carries.
pub(crate) const MERCHANT_AUDIENCE_CLAIM: &str = "merchant";

/// Who an authenticated merchant MCP request acts as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCaller {
    /// The tenant.
    pub merchant_id: String,
    /// Whether the token holds `payments:write`.
    pub can_write: bool,
}

/// Why a request was not authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpAuthRefusal {
    /// The token is not one this endpoint accepts (bad signature, expired,
    /// another audience, an unknown client). Answered `401`.
    InvalidToken(String),
    /// The token is valid and holds no scope this endpoint serves. Answered
    /// `403` with `insufficient_scope`, so a client knows to ask for more
    /// rather than to sign in again.
    InsufficientScope(String),
    /// The verifier could not decide (its key set is unreachable). Answered
    /// as a server error, so a client is not sent to re-authenticate for
    /// nothing.
    Unavailable(String),
}

/// Authenticates the bearer token of a merchant MCP request. Implemented by
/// `vpay-api`.
#[async_trait]
pub trait McpAuthenticator: Send + Sync {
    /// Verifies `token`, which the transport has already taken out of
    /// `Authorization: Bearer`.
    ///
    /// # Errors
    ///
    /// [`McpAuthRefusal`] for any token that does not authenticate.
    async fn authenticate(&self, token: &str) -> Result<McpCaller, McpAuthRefusal>;
}

/// Where the endpoint is and who may call it: the protected-resource
/// identifier MCP clients see (RFC 9728), its authorization server, and the
/// browser origins the transport accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEndpoint {
    /// The resource identifier, an absolute URL ending at the mount path.
    pub resource: String,
    /// The issuer MCP clients get tokens from.
    pub authorization_servers: Vec<String>,
    /// The scopes listed as `scopes_supported`.
    pub scopes: Vec<String>,
    /// Browser origins that may call the endpoint. CrateStack refuses an
    /// empty list.
    pub allowed_origins: Vec<String>,
}

/// The endpoint configuration CrateStack refused when the router was built:
/// an empty origin list, a resource identifier that is not an absolute URL,
/// and the like. A boot failure, never a request's.
#[derive(Debug, thiserror::Error)]
#[error("the merchant MCP endpoint could not be built: {0}")]
pub struct McpEndpointError(pub(crate) String);

impl vpay_core::Classify for McpEndpointError {
    fn category(&self) -> vpay_core::Category {
        vpay_core::Category::Configuration
    }
}

/// [`cratestack::AuthProvider`] over an [`McpAuthenticator`].
#[derive(Clone)]
pub(crate) struct MerchantMcpProvider(pub(crate) Arc<dyn McpAuthenticator>);

impl cratestack::AuthProvider for MerchantMcpProvider {
    type Error = CratestackError;

    async fn authenticate(
        &self,
        request: &cratestack::RequestContext<'_>,
    ) -> Result<CratestackContext, Self::Error> {
        // The transport refuses a request without exactly one well-formed
        // `Authorization: Bearer` before calling a provider, so a missing
        // token here is a wiring defect, not a caller's mistake.
        let token = request
            .headers
            .get(cratestack::axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim())
            .ok_or_else(|| {
                CratestackError::Unauthorized("no bearer token reached the provider".to_owned())
            })?;
        match self.0.authenticate(token).await {
            Ok(caller) => Ok(merchant_context(&caller)),
            Err(McpAuthRefusal::InvalidToken(detail)) => Err(CratestackError::Unauthorized(detail)),
            Err(McpAuthRefusal::InsufficientScope(detail)) => {
                Err(CratestackError::Forbidden(detail))
            }
            Err(McpAuthRefusal::Unavailable(detail)) => Err(CratestackError::Unavailable(detail)),
        }
    }
}

/// The context a merchant MCP request runs under. See the module doc's
/// table.
pub(crate) fn merchant_context(caller: &McpCaller) -> CratestackContext {
    CratestackContext::with_principal(PrincipalContext {
        tenant: Some(PrincipalFacet {
            fields: BTreeMap::from([("id".to_owned(), Value::String(caller.merchant_id.clone()))]),
        }),
        claims: BTreeMap::from([
            (
                "audience".to_owned(),
                Value::String(MERCHANT_AUDIENCE_CLAIM.to_owned()),
            ),
            ("can_write".to_owned(), Value::Bool(caller.can_write)),
        ]),
        ..PrincipalContext::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_merchant_context_carries_the_tenant_audience_and_write_claim_and_is_never_system() {
        let ctx = merchant_context(&McpCaller {
            merchant_id: "acct_1".to_owned(),
            can_write: false,
        });
        assert_eq!(ctx.tenant_id(), Some("acct_1"));
        assert_eq!(
            ctx.auth_field("audience"),
            Some(&Value::String("merchant".to_owned()))
        );
        assert_eq!(ctx.auth_field("can_write"), Some(&Value::Bool(false)));
        assert!(ctx.is_authenticated());
        assert!(!ctx.is_system());
    }
}
