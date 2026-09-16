//! The dashboard's CrateStack auth seam (Lane C,
//! `docs/plans/2026-09-13-dashboard-nav-notes/transport.md`).
//!
//! `cratestack::AuthProvider` (`cratestack-core-0.12.0/src/context.rs:92`)
//! has a blanket impl for any `Fn(&http::HeaderMap) -> Result<CratestackContext,
//! E>`, which is the shape most consumers reach for. This crate cannot use
//! it: `require_dashboard_procedure_token` (`vpay-api`) has *already*
//! validated the caller's bearer token, re-read the staff row and resolved
//! `vpay_api::dash::DashboardTenancy` by the time this transport's own
//! request reaches it — repeating any of that here would be a second,
//! parallel decision about tenancy rather than a use of the one lane B
//! already made. What that decision produced is stashed on the request's
//! *extensions*, not its headers, so the blanket impl's signature cannot
//! read it. [`ExtensionAuthProvider`] implements the trait directly instead.
//!
//! # What this module must never do
//!
//! Mint [`crate::persistence::system_context`]. That context is the only one
//! in this crate for which `is_system()` is true, and every `@allow` arm this
//! schema declares that reads `auth().isSystem()` would pass for a request
//! built from it. [`ExtensionAuthProvider::authenticate`] builds a context
//! with [`cratestack::CratestackContext::with_principal`], never
//! `system_context()`, and there is no path from one to the other —
//! `SystemContext` has no `From<CratestackContext>` and no constructor that
//! accepts an existing one (`cratestack-core-0.12.0/src/context.rs`'s own
//! doc on the `system` field says so).
//!
//! # Why this reads a tenant string, not a `vpay_api::dash::DashboardTenancy`
//!
//! `vpay-api` depends on this crate; this crate must not depend back on
//! `vpay-api` (a cycle), and `cratestack` is a dependency of this crate
//! alone in the workspace
//! (`docs/reference/vpay-db/cratestack.md` § CrateStack) — so `vpay-api`
//! must never import `cratestack::CratestackContext`/`CratestackError`
//! either, or that boundary is gone in the other direction. [`DashboardAuthFn`]
//! is the seam that keeps both halves true: `vpay-api` hands over a plain
//! `axum::http::Extensions -> Option<String>` closure that knows about
//! `DashboardTenancy` but nothing about CrateStack, and this module is the
//! only place that turns "a tenant string, or none" into a
//! [`cratestack::CratestackContext`].

use std::sync::Arc;

use cratestack::axum::http::Extensions;
use cratestack::{CratestackContext, CratestackError, PrincipalContext, PrincipalFacet, Value};

/// Reads the tenant a validated dashboard request may act as out of the
/// request's own `Extensions`, or `None` if nothing put one there.
///
/// `vpay-api` is the only crate that constructs one, from a closure that
/// reads `vpay_api::dash::DashboardTenancy` (inserted into the request's
/// extensions by `require_dashboard_procedure_token` before this transport
/// ever runs) — see this module's doc for why the closure returns a plain
/// tenant id and not a `CratestackContext` itself.
///
/// `Arc`, not a bare `fn` or a generic closure type, so
/// [`ExtensionAuthProvider`] stays `Clone` — `cratestack::AuthProvider`
/// requires it — without requiring the caller's closure to be `Clone`
/// itself.
pub type DashboardAuthFn = Arc<dyn Fn(&Extensions) -> Option<String> + Send + Sync>;

/// [`cratestack::AuthProvider`] over a [`DashboardAuthFn`].
///
/// A tenant-carrying context — never an anonymous or a system one — for any
/// request the closure resolves a tenant for; [`CratestackError::Internal`]
/// (a `500`, never told to the caller as anything specific) when it does
/// not, because that means the request reached this transport with no
/// `require_dashboard_procedure_token` in front of it — a wiring defect, the
/// same fail-closed shape `vpay_api::dash::DashboardTenancy`'s own
/// `FromRequestParts` impl answers for the identical reason.
#[derive(Clone)]
pub(crate) struct ExtensionAuthProvider(pub(crate) DashboardAuthFn);

impl cratestack::AuthProvider for ExtensionAuthProvider {
    type Error = CratestackError;

    fn authenticate(
        &self,
        request: &cratestack::RequestContext<'_>,
    ) -> impl core::future::Future<Output = Result<CratestackContext, Self::Error>> + Send {
        let tenant = (self.0)(request.extensions);
        let result = match tenant {
            Some(tenant_id) => Ok(CratestackContext::with_principal(PrincipalContext {
                tenant: Some(PrincipalFacet {
                    fields: std::collections::BTreeMap::from([(
                        "id".to_owned(),
                        Value::String(tenant_id),
                    )]),
                }),
                ..PrincipalContext::default()
            })),
            None => Err(CratestackError::Internal(
                "a dashboard procedure request ran with no tenant resolved on its extensions: \
                 require_dashboard_procedure_token is not mounted in front of this transport"
                    .to_owned(),
            )),
        };
        core::future::ready(result)
    }
}

#[cfg(test)]
mod tests {
    use cratestack::AuthProvider as _;

    use super::*;

    fn request_context_over<'a>(
        extensions: &'a Extensions,
        headers: &'a cratestack::axum::http::HeaderMap,
    ) -> cratestack::RequestContext<'a> {
        cratestack::RequestContext {
            method: "POST",
            path: "/$procs/searchPaymentIntents",
            query: None,
            headers,
            body: &[],
            extensions,
        }
    }

    /// A closure resolving a tenant produces an authenticated,
    /// tenant-carrying context — never a system one.
    #[tokio::test]
    async fn a_resolved_tenant_becomes_an_authenticated_tenant_carrying_context() {
        let provider = ExtensionAuthProvider(Arc::new(|_: &Extensions| {
            Some("acme-cameroon-tenant".to_owned())
        }));
        let extensions = Extensions::new();
        let headers = cratestack::axum::http::HeaderMap::new();
        let ctx = provider
            .authenticate(&request_context_over(&extensions, &headers))
            .await
            .expect("a resolved tenant is not refused");

        assert_eq!(ctx.tenant_id(), Some("acme-cameroon-tenant"));
        assert!(ctx.is_authenticated());
        assert!(
            !ctx.is_system(),
            "this seam must never produce a context for which is_system() is true"
        );
    }

    /// No tenant on the extensions — the "middleware not mounted" case —
    /// fails closed with an internal error rather than an anonymous or
    /// system context.
    #[tokio::test]
    async fn no_resolved_tenant_fails_closed() {
        let provider = ExtensionAuthProvider(Arc::new(|_: &Extensions| None));
        let extensions = Extensions::new();
        let headers = cratestack::axum::http::HeaderMap::new();
        let error = provider
            .authenticate(&request_context_over(&extensions, &headers))
            .await
            .expect_err("no tenant on the request must not be served");
        assert!(matches!(error, CratestackError::Internal(_)), "{error:?}");
    }
}
