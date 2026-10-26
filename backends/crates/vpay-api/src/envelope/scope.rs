//! From a verified signer to a tenant and its scopes (VP-3).
//!
//! The envelope proves **who holds the key** and authenticates no one
//! (cratestack ADR 0006: "the envelope does not authenticate"). This is the
//! step that says what that key may do, and it is the whole of what
//! `require_merchant_token` was for `/v1` before: resolve the tenant, check the
//! scope the method needs, put a [`MerchantScope`] on the request.
//!
//! It runs **inside** the envelope layer, so it sees the request the layer
//! opened and the [`VerifiedSigner`] it recorded, and its refusals are sealed
//! like any handler's. It runs before the router matches a route's handler,
//! which keeps the property the old middleware had and the reason it was a
//! middleware: a `/v1` handler cannot be added without it. A handler that asks
//! for [`MerchantScope`] and runs without this step gets a `500`
//! (`MerchantScope`'s extractor), never another tenant.
//!
//! # The two answers, and why neither is a 401
//!
//! | Situation | Answer |
//! |---|---|
//! | no [`VerifiedSigner`] on the request | `500` — the layer is not in front of this route |
//! | the key resolves to no enabled merchant | `403` `forbidden`, sealed |
//! | the merchant's YAML scopes carry none the method needs | `403` `forbidden`, sealed |
//! | the `disabled_clients` lookup failed | the database's own error, sealed |
//!
//! A *disabled* client never gets this far: [`MerchantKeyResolver`] resolves it
//! to no key, so its signature does not verify and the answer is the layer's
//! coarse, unsigned `401`, indistinguishable from an unknown key. The `403`
//! here is for the window between the two lookups (the client was disabled
//! after its key resolved) and for the scope check, and — unlike a bearer token
//! — it is *sealed*, because the signature did verify and the merchant may
//! trust "you are not permitted".
//!
//! # Object tenancy is not decided here
//!
//! A statement about the *credential* is a `403`; a statement about an
//! *object* is a uniform `404` from the handler, filtered by the
//! [`MerchantScope`] inserted below. That split is `vpay-merchant-api`'s rule
//! and unchanged.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cratestack_core::VerifiedSigner;

use crate::ApiError;
use crate::merchant_keys::MerchantKeyResolver;
use crate::v1::{MerchantScope, required_scopes};

/// The middleware. Mounted with
/// `from_fn_with_state(Arc<MerchantKeyResolver>, merchant_scope)` directly
/// around the `/v1` routes, inside the envelope.
pub(crate) async fn merchant_scope(
    State(merchants): State<Arc<MerchantKeyResolver>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();

    let Some(signer) = parts.extensions.get::<VerifiedSigner>() else {
        return ApiError::Internal(
            "a /v1 request reached the merchant scope step with no VerifiedSigner: the \
             envelope layer is not mounted in front of this route"
                .to_owned(),
        )
        .into_response();
    };

    let merchant = match merchants.merchant_for_thumbprint(signer.thumbprint()).await {
        Ok(Some(merchant)) => merchant,
        Ok(None) => {
            tracing::warn!(
                "a verified /v1 signature names a key that no enabled merchant holds (it was \
                 disabled after the key resolved); refusing rather than guessing a tenant"
            );
            return ApiError::Forbidden.into_response();
        }
        // A failed lookup is an error, not a miss: admitting the request would
        // make a database outage a revocation bypass.
        Err(error) => return ApiError::from(error).into_response(),
    };

    let required = required_scopes(&parts.method);
    if !required
        .iter()
        .any(|scope| merchant.scopes.iter().any(|granted| granted == scope))
    {
        tracing::warn!(
            client_id = %merchant.client_id,
            method = %parts.method,
            granted = ?merchant.scopes,
            required = ?required,
            "a /v1 merchant registration carries none of the scopes this request needs; refusing"
        );
        return ApiError::Forbidden.into_response();
    }

    parts
        .extensions
        .insert(MerchantScope::for_signed(merchant.merchant_id));
    next.run(Request::from_parts(parts, body)).await
}
