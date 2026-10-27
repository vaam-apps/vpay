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
//! # Two credentials, one per request (ADR-0037)
//!
//! `auth.methods` decides which ways in exist. A request that verified under a
//! registered key is a **signer** and resolves through the merchant key
//! registry. One that did not may be a **token caller** when `machine_tokens`
//! is permitted: its bearer token is validated (audience `vpay:v1`, this
//! deployment's issuer) and its `client_id` resolves to a tenant through the
//! YAML, exactly as the bearer middleware did before VP-3. A request that is
//! both is refused `400`: one caller, one credential.
//!
//! | Situation | Answer |
//! |---|---|
//! | a [`VerifiedSigner`] **and** an `Authorization` header | `400`, sealed |
//! | no [`VerifiedSigner`], no `Authorization`, `machine_tokens` permitted | `401` `missing_bearer_token`, plain |
//! | no [`VerifiedSigner`], a token that does not validate | `401` `invalid_token`, plain; `503` when the JWKS cannot be read |
//! | no [`VerifiedSigner`], a valid token naming no registered client | `403` `forbidden`, plain |
//! | a valid token whose scopes carry none the method needs | `403` `forbidden`, plain |
//!
//! A token caller's answers are plain JSON, as they were before VP-3; they are
//! unsealed and not bound to a contract (the `426` for an operation whose
//! shape is gone is a sealed caller's protection), which is the trade a
//! deployment makes by permitting the method.
//!
//! # The other answers, and why none is a 401
//!
//! | Situation | Answer |
//! |---|---|
//! | no [`VerifiedSigner`], method `OPTIONS` | axum's own `405` with `Allow`: the one request the layer lets through unsigned (a CORS preflight), and no `/v1` handler answers it |
//! | no [`VerifiedSigner`], any other method, `machine_tokens` not permitted | `500` — the layer is not in front of this route |
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
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cratestack_core::VerifiedSigner;

use crate::ApiError;
use crate::merchant_keys::MerchantKeyResolver;
use crate::resource_auth::{JwtValidator, extract_bearer_token};
use crate::v1::{MerchantScope, ResourceConfig, required_scopes};

/// The `auth.method` span field's two values on `/v1` (ADR-0037, tracing
/// only: no policy reads it).
const AUTH_METHOD_FIELD: &str = "auth.method";
const AUTH_METHOD_KEYPAIR: &str = "keypair";
const AUTH_METHOD_MACHINE_TOKEN: &str = "machine_token";

/// What the scope step reads: the signer registry, and — when `auth.methods`
/// permits `machine_tokens` — the bearer-token validator and the YAML map from
/// a token's `client_id` to its tenant.
#[derive(Clone)]
pub(crate) struct ScopeState {
    pub(crate) merchants: Arc<MerchantKeyResolver>,
    pub(crate) machine_tokens: bool,
    pub(crate) validator: Option<JwtValidator>,
    pub(crate) resource_config: Arc<ResourceConfig>,
}

/// The middleware. Mounted with `from_fn_with_state(ScopeState, merchant_scope)`
/// directly around the `/v1` routes, inside the envelope.
pub(crate) async fn merchant_scope(
    State(state): State<ScopeState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();

    let signer = parts.extensions.get::<VerifiedSigner>().cloned();
    let has_authorization = parts.headers.contains_key(header::AUTHORIZATION);

    let Some(signer) = signer else {
        // The one request the layer lets through with no signature, under
        // `Required`, is a bodiless `OPTIONS` (cratestack 0.16.0 passes it for
        // a CORS preflight). `route_layer` wraps a method router's own `405`
        // fallback, so it arrives here; no `/v1` handler answers `OPTIONS`,
        // so what it reaches is axum's `405` with `Allow`, which is the honest
        // answer to a method the surface does not serve and which no browser
        // caller is expected to need (`/v1` is a merchant's backend; a payer's
        // page uses `/v1/browser`). It is NOT the misconfiguration below: an
        // anonymous caller must not be able to raise a page by asking.
        if parts.method == Method::OPTIONS {
            return next.run(Request::from_parts(parts, body)).await;
        }
        if !state.machine_tokens {
            return ApiError::Internal(
                "a /v1 request reached the merchant scope step with no VerifiedSigner: the \
                 envelope layer is not mounted in front of this route"
                    .to_owned(),
            )
            .into_response();
        }
        return token_scope(&state, parts, body, next).await;
    };

    // One caller, one credential: a request that proved a key and also
    // presents a bearer token is ambiguous about who is asking, and the
    // answer to ambiguity is to refuse, not to pick.
    if has_authorization {
        return ApiError::invalid_param(
            "authorization",
            "A request is authenticated by its signature or by a bearer token, never both. \
             Remove the Authorization header from a signed request.",
        )
        .into_response();
    }

    let merchant = match state
        .merchants
        .merchant_for_thumbprint(signer.thumbprint())
        .await
    {
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

    tracing::Span::current().record(AUTH_METHOD_FIELD, AUTH_METHOD_KEYPAIR);
    parts
        .extensions
        .insert(MerchantScope::for_signed(merchant.merchant_id));
    next.run(Request::from_parts(parts, body)).await
}

/// The bearer-token half: an unsigned request on a deployment that permits
/// `machine_tokens`. It is what `require_merchant_token` was for `/v1` until
/// VP-3 — validate once, map the `client_id` to a tenant, check the scope the
/// method needs — with a `403` rather than a `401` for a genuine token that
/// names no registered client, for the reason that middleware gave: the
/// credential is real, and there is no tenant it may act on.
async fn token_scope(
    state: &ScopeState,
    mut parts: axum::http::request::Parts,
    body: Body,
    next: Next,
) -> Response {
    let Some(validator) = &state.validator else {
        return ApiError::Internal(
            "auth.methods permits machine_tokens but this router was assembled with no bearer \
             token validator for /v1; refusing every unsigned request"
                .to_owned(),
        )
        .into_response();
    };
    let claims = match extract_bearer_token(&parts) {
        Ok(token) => match validator.validate(token).await {
            Ok(claims) => claims,
            Err(rejection) => return ApiError::from(rejection).into_response(),
        },
        Err(rejection) => return ApiError::from(rejection).into_response(),
    };

    let Some(merchant_id) = state.resource_config.merchant_id_for(&claims.subject) else {
        tracing::warn!(
            client_id = %claims.subject,
            "a validly signed /v1 token names a client this deployment has no registration for; \
             refusing rather than guessing a tenant"
        );
        return ApiError::Forbidden.into_response();
    };
    let scope = MerchantScope::for_token(merchant_id.to_owned());

    let required = required_scopes(&parts.method);
    if !required.iter().any(|scope| claims.has_scope(scope)) {
        tracing::warn!(
            client_id = %claims.subject,
            method = %parts.method,
            granted = ?claims.scope,
            required = ?required,
            "a /v1 token carries none of the scopes this request needs; refusing"
        );
        return ApiError::Forbidden.into_response();
    }

    tracing::Span::current().record(AUTH_METHOD_FIELD, AUTH_METHOD_MACHINE_TOKEN);
    parts.extensions.insert(claims);
    parts.extensions.insert(scope);
    next.run(Request::from_parts(parts, body)).await
}
