//! From a verified service key to the capability a `/svc/v1` route needs (VP-6a).
//!
//! The envelope proves who holds the key and authenticates no one (cratestack ADR
//! 0006: "the envelope does not authenticate"). This is the step that says what
//! that key is worth, and it is the whole of what stands between a signed
//! request and a pairing:
//!
//! 1. the signer resolves to a **service** (`service_clients[].cose_keys`, not
//!    disabled) — a merchant's key never gets this far, because the layer's
//!    resolver is the service registry alone;
//! 2. the matched route's row in [`crate::svc::SVC_ROUTES`] names a capability;
//! 3. the service holds it. If not: `403`, sealed.
//!
//! Only then is a [`PairingAuthority`] put on the request, and the handler
//! demands one. A capability is configuration; nothing here, and no route
//! anywhere, adds one.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{MatchedPath, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cratestack_core::VerifiedSigner;
use vpay_config::ServiceCapability;

use crate::ApiError;
use crate::merchant_keys::ServiceKeyResolver;
use crate::svc::{PairingAuthority, capability_for};

/// The middleware. Mounted with `route_layer` directly around the `/svc/v1`
/// routes, inside the envelope.
///
/// | Situation | Answer |
/// |---|---|
/// | no `VerifiedSigner`, method `OPTIONS` | the router's own `405` (see `dash_scope`) |
/// | no `VerifiedSigner`, any other method | `500`: the layer is not in front of this route |
/// | the key resolves to no enabled service (disabled after it resolved) | `403` |
/// | the route has no capability row | `500`: a route the scope cannot guard |
/// | the service lacks the route's capability | `403` |
pub(crate) async fn service_scope(
    State(services): State<Arc<ServiceKeyResolver>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();

    let Some(signer) = parts.extensions.get::<VerifiedSigner>() else {
        if parts.method == Method::OPTIONS {
            return next.run(Request::from_parts(parts, body)).await;
        }
        return ApiError::Internal(
            "a /svc/v1 request reached the service scope step with no VerifiedSigner: the \
             envelope layer is not mounted in front of this route"
                .to_owned(),
        )
        .into_response();
    };

    let service = match services.service_for_thumbprint(signer.thumbprint()).await {
        Ok(Some(service)) => service,
        Ok(None) => {
            tracing::warn!(
                "a verified /svc/v1 signature names a key that no enabled service holds (it was \
                 disabled after the key resolved); refusing rather than guessing"
            );
            return ApiError::Forbidden.into_response();
        }
        // A failed lookup is an error, not a miss.
        Err(error) => return ApiError::from(error).into_response(),
    };

    let required: Option<ServiceCapability> = parts
        .extensions
        .get::<MatchedPath>()
        .and_then(|matched| capability_for(matched.as_str(), parts.method.as_str()));
    let Some(required) = required else {
        return ApiError::Internal(
            "a /svc/v1 route has no capability row in SVC_ROUTES: the scope step cannot guard it"
                .to_owned(),
        )
        .into_response();
    };
    if !service.holds(required) {
        tracing::warn!(
            client_id = %service.client_id,
            required = required.as_str(),
            "a /svc/v1 service lacks the capability this route needs; refusing"
        );
        return ApiError::Forbidden.into_response();
    }

    // One authority type per capability, minted by an exhaustive `match`: a
    // capability added to the vocabulary is a compile error here until it says
    // what the handlers behind it are handed.
    match required {
        ServiceCapability::ConsolePairing => {
            parts
                .extensions
                .insert(PairingAuthority::granted_to(service.client_id));
        }
    }
    next.run(Request::from_parts(parts, body)).await
}
