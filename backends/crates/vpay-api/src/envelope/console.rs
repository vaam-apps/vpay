//! A person's console, as the signed transport sees it (VP-6a): the resolver
//! that finds the P-256 keys a signature may verify under, and the step that
//! turns a verified key into the one tenant `/dash/v1` reads.
//!
//! ```text
//!   COSE_Sign1 ──▶ EnvelopeLayer ──asks──▶ ConsoleKeyResolver ──▶ console_keys
//!                       │                  (live: not revoked, not expired)
//!                       ▼ verified, `VerifiedSigner` recorded
//!                  dash_scope ── key row ─▶ staff row ─▶ DashboardTenancy
//!                       │         (live?)    (active, this merchant?)
//!                       ▼
//!                    handler  (reads exactly the tenant the binding names)
//! ```
//!
//! # Two doors, one tenant (ADR-0037)
//!
//! On a deployment that serves `human_login` the layer runs `Optional`, and this
//! step decides between the two credentials a request can carry. A request the
//! layer opened is a **key** request ([`VerifiedSigner`] on its extensions, which
//! only the layer sets after the signature verified); one it let through
//! untouched is a **session** request, and must present the staff access token
//! the authorization-code grant mints (`Authorization: Bearer`, audience
//! `dashboard_client.client_id`). Both end in the same place: an active member of
//! the dashboard's tenant, the same admin override rules, and one
//! [`DashboardTenancy`] on the request. A request that is both is refused `400`
//! (sealed, because the signature verified): one caller, one credential.
//!
//! | Situation | Answer |
//! |---|---|
//! | a [`VerifiedSigner`] **and** an `Authorization` header | `400` naming `authorization`, sealed |
//! | a [`VerifiedSigner`] only | the key path below |
//! | neither, `human_login` served, a valid dashboard token | the session path below |
//! | neither, `human_login` served, no token or a bad one | `401`, plain |
//! | neither, `human_login` not served | `500`: the layer is not in front of this route |
//!
//! # Revocation takes effect on the next request, and expiry on the instant
//!
//! [`ConsoleKeyResolver`] reads `console_keys` **on every request**: there is
//! no cache anywhere between the table and the signature check. A revoked key
//! and an expired key (`expires_at <= now`) resolve to no key, so the layer
//! answers the coarse, unsigned `401` that every other refusal is — the same
//! bytes for an unknown key, a revoked one, an expired one and a bad signature,
//! so the response is no oracle for which. The cost, accepted on purpose, is one
//! indexed read per management request; the management surface is a handful of
//! people.
//!
//! A database that cannot answer is a `500`, never a miss: admitting a request
//! when the lookup failed would make an outage a revocation bypass.
//!
//! # Tenancy is fixed by the binding
//!
//! A non-administrator reads `dashboard.merchant_id` and nothing the request
//! says: the `?merchant_id=` parameter is not even parsed for them. An
//! administrator (made by `vpay-server staff add --admin`, keyed by `staff
//! pair-key` — never by a pairing, which refuses one) may name another
//! registered merchant (ADR-0018). Both rules are the old bearer-token step's,
//! unchanged; what changed is what proves who the person is.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cratestack_core::{CratestackError, VerifiedSigner};
use cratestack_cose::thumbprint::KID_LEN;
use cratestack_cose::{CoseAlg, CoseVerifierResolver, CoseVerifyKey};
use time::OffsetDateTime;
use vpay_db::Repositories;

use vpay_db::StaffRow;

use crate::dash::{AdminMerchantOverride, DashboardTenancy};
use crate::resource_auth::{JwtValidator, extract_bearer_token};
use crate::v1::{DashboardBinding, MerchantScope};
use crate::{ApiError, ResourceConfig};

/// Finds the P-256 keys a signature's `kid` may name, among the live rows of
/// `console_keys`.
///
/// Cheap to clone: one `Arc`.
#[derive(Clone)]
pub struct ConsoleKeyResolver {
    repositories: Arc<dyn Repositories>,
}

impl std::fmt::Debug for ConsoleKeyResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsoleKeyResolver").finish_non_exhaustive()
    }
}

impl ConsoleKeyResolver {
    /// A resolver over the process's repositories.
    #[must_use]
    pub fn new(repositories: Arc<dyn Repositories>) -> Self {
        Self { repositories }
    }
}

#[async_trait]
impl CoseVerifierResolver for ConsoleKeyResolver {
    /// Every live key whose thumbprint starts with `kid`, as a verification key.
    ///
    /// An unknown, revoked or expired `kid` is `Ok(vec![])`, never `Err` (the
    /// trait's contract: an `Err` is a `500`, and answering "no such key" with
    /// one would be the oracle ADR 0006 §10 forbids). A lookup that *fails* is
    /// `Err(Internal)`, because the other reading — "no key" — would make a
    /// database outage look like a revocation.
    async fn resolve(
        &self,
        kid: &[u8],
        alg: CoseAlg,
    ) -> Result<Vec<CoseVerifyKey>, CratestackError> {
        if kid.len() != KID_LEN {
            return Ok(Vec::new());
        }
        let rows = self
            .repositories
            .live_console_keys_for_kid(&hex::encode(kid), OffsetDateTime::now_utc())
            .await
            .map_err(|error| {
                tracing::error!(%error, "the console key lookup failed; refusing the request");
                CratestackError::Internal("the console key lookup failed".to_owned())
            })?;
        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            let decoded = URL_SAFE_NO_PAD
                .decode(&row.public_key)
                .ok()
                .and_then(|bytes| CoseVerifyKey::p256_sec1(&bytes).ok());
            match decoded {
                Some(key) if key.supports(alg) => keys.push(key),
                Some(_) => {}
                None => tracing::error!(
                    thumbprint = %row.thumbprint,
                    "a stored console key does not decode as a P-256 point; skipping it"
                ),
            }
        }
        Ok(keys)
    }
}

/// Which of the two `/dash/v1` routers a scope step guards: it decides which
/// HTTP method a session may use. A key's methods are the layer's own (the
/// binding lists them), but an unsigned request is never opened by the layer, so
/// the same refusal has to be made here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashSurface {
    /// The hand-written reads ([`crate::DASH_ROUTES`]): `GET` and `HEAD` only.
    Reads,
    /// CrateStack's procedure transport: `POST` only.
    Procedures,
}

impl DashSurface {
    /// Whether a session (a request the layer did not open) may use `method`
    /// here. Everything else is refused `403` before the token is looked at, so
    /// a write is never *authenticated* here, only rejected (ADR-0008).
    fn serves(self, method: &Method) -> bool {
        match self {
            // `HEAD` alongside `GET` for `v1::required_scopes`' reason: axum
            // answers it from the same `get(..)` handler.
            Self::Reads => matches!(*method, Method::GET | Method::HEAD),
            Self::Procedures => *method == Method::POST,
        }
    }
}

/// What [`dash_scope`] needs: the repositories to re-read the key and the
/// member, the configuration that names the one tenant, and, when `human_login`
/// is served, the validator that checks a session's access token.
#[derive(Clone)]
pub(crate) struct DashScopeState {
    pub(crate) repositories: Arc<dyn Repositories>,
    pub(crate) resource_config: Arc<ResourceConfig>,
    pub(crate) surface: DashSurface,
    /// `Some` with `human_login` served. `None` while the configuration serves
    /// it is a router assembled without one, and every unsigned request is then
    /// a `500`, never an open door.
    pub(crate) sessions: Option<JwtValidator>,
}

/// The `auth.method` span field's values on `/dash/v1` (ADR-0037, tracing only:
/// no policy reads it).
const AUTH_METHOD_FIELD: &str = "auth.method";
const AUTH_METHOD_KEYPAIR: &str = "keypair";
const AUTH_METHOD_HUMAN_SESSION: &str = "human_session";

/// The middleware. Mounted with `route_layer` directly around the `/dash/v1`
/// routes and the procedure routes, **inside** the envelope.
///
/// Refusals of the key path, all sealed because the signature verified:
///
/// | Situation | Answer |
/// |---|---|
/// | no `VerifiedSigner`, method `OPTIONS` | the router's own `405` (a CORS preflight the layer lets through unsigned; no console route answers it) |
/// | no `VerifiedSigner`, `human_login` served | the session path ([`session_scope`]) |
/// | no `VerifiedSigner`, any other method | `500`: the layer is not in front of this route |
/// | a `VerifiedSigner` and an `Authorization` header | `400` naming `authorization` |
/// | the key was revoked or expired between the resolver's read and this one | `403` |
/// | the member is disabled, gone, or of another merchant than the dashboard's | `403`, one answer for all three |
/// | an administrator names an unknown merchant | `400` naming `merchant_id` |
pub(crate) async fn dash_scope(
    State(state): State<DashScopeState>,
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
        // fallback, so it arrives here; no console route answers `OPTIONS`, so
        // what it reaches is axum's `405` with `Allow`. It is NOT the
        // misconfiguration below: an anonymous caller must not be able to
        // raise a page by asking.
        if parts.method == Method::OPTIONS {
            return next.run(Request::from_parts(parts, body)).await;
        }
        if state.resource_config.staff_binding().is_none() {
            return ApiError::Internal(
                "a /dash/v1 request reached the console scope step with no VerifiedSigner: the \
                 envelope layer is not mounted in front of this route"
                    .to_owned(),
            )
            .into_response();
        }
        return session_scope(&state, parts, body, next).await;
    };

    // One caller, one credential: a request that proved a key and also presents
    // a bearer token is ambiguous about who is asking, and the answer to
    // ambiguity is to refuse, not to pick.
    if has_authorization {
        return ApiError::invalid_param(
            "authorization",
            "A request is authenticated by its signature or by a bearer token, never both. \
             Remove the Authorization header from a signed request.",
        )
        .into_response();
    }

    let Some(binding) = state.resource_config.dashboard() else {
        return ApiError::UnknownRoute {
            method: parts.method.to_string(),
            path: parts.uri.path().to_owned(),
        }
        .into_response();
    };

    let thumbprint = hex::encode(signer.thumbprint());
    let now = OffsetDateTime::now_utc();
    let key = match state.repositories.find_console_key(&thumbprint).await {
        Ok(Some(key)) if key.is_live(now) => key,
        Ok(_) => {
            tracing::warn!(
                %thumbprint,
                "a verified /dash/v1 signature names a console key that was revoked or expired \
                 after it resolved; refusing"
            );
            return ApiError::Forbidden.into_response();
        }
        // A failed lookup is an error, not a miss: admitting the request would
        // make a database outage a revocation bypass.
        Err(error) => return ApiError::from(error).into_response(),
    };

    // THE STAFF ROW IS RE-READ ON EVERY REQUEST, so disabling a person or moving
    // them to another merchant takes effect on their next request. One answer
    // for "disabled", "gone" and "no longer this tenant's": telling them apart
    // would say which `stf_…` values name a row.
    let staff = match state.repositories.find(&key.staff_member_id).await {
        Ok(Some(staff)) if staff.is_active() && staff.merchant_id == binding.merchant_id => staff,
        Ok(_) => {
            tracing::warn!(
                staff_id = %key.staff_member_id,
                "a verified /dash/v1 key belongs to a staff member who is disabled, gone, or no \
                 longer this deployment's merchant's; refusing"
            );
            return ApiError::Forbidden.into_response();
        }
        Err(error) => {
            // Fail closed. A database that cannot answer "is this person still
            // allowed in" must not be read as "yes".
            tracing::error!(%error, "reading the staff row behind a /dash/v1 key");
            return ApiError::from(error).into_response();
        }
    };

    let tenancy = match resolve_tenancy(&mut parts, &state, &staff, binding).await {
        Ok(tenancy) => tenancy,
        Err(response) => return *response,
    };

    tracing::Span::current().record(AUTH_METHOD_FIELD, AUTH_METHOD_KEYPAIR);
    parts.extensions.insert(MerchantScope::for_dashboard(
        tenancy.merchant_id().to_owned(),
    ));
    parts.extensions.insert(tenancy);
    next.run(Request::from_parts(parts, body)).await
}

/// The session half: an unsigned request on a deployment that serves
/// `human_login`. It is what `require_dashboard_token` was for `/dash/v1` until
/// VP-6a: validate the staff access token once, check the tenant claim and the
/// registration's scope, re-read the member, and resolve ADR-0018's tenancy.
///
/// | Situation | Answer |
/// |---|---|
/// | a method this router does not serve | `403`, before the token is looked at |
/// | no token, or one that does not validate (signature, expiry, issuer, audience) | `401`, plain; `503` when the JWKS cannot be read |
/// | a valid token with no `vpay_merchant_id` claim, or another merchant's | `403`: a `client_credentials` token has none, so no machine client reads this surface |
/// | a valid token without the registration's scope | `403` |
/// | the member named by `sub` is disabled, gone, or of another merchant | `403`, one answer for all three |
/// | the database cannot answer | its own error; never a pass |
///
/// A session's answers are plain JSON, as the key path's are sealed: the token
/// caller gets no contract-lock protection, which is the trade a deployment
/// makes by serving the method.
async fn session_scope(
    state: &DashScopeState,
    mut parts: axum::http::request::Parts,
    body: Body,
    next: Next,
) -> Response {
    let (Some(validator), Some(staff_binding), Some(binding)) = (
        &state.sessions,
        state.resource_config.staff_binding(),
        state.resource_config.dashboard(),
    ) else {
        return ApiError::Internal(
            "auth.methods serves human_login but this router was assembled with no session \
             token validator for /dash/v1; refusing every unsigned request"
                .to_owned(),
        )
        .into_response();
    };

    // Before the token is even looked at: a method this surface does not serve
    // is refused whatever credential it carries (ADR-0008: no write reaches a
    // handler without an audit log, and none exists).
    if !state.surface.serves(&parts.method) {
        tracing::warn!(
            method = %parts.method,
            "a /dash/v1 request names a method this read-only surface does not serve; refusing"
        );
        return ApiError::Forbidden.into_response();
    }

    let claims = match extract_bearer_token(&parts) {
        Ok(token) => match validator.validate(token).await {
            Ok(claims) => claims,
            Err(rejection) => return ApiError::from(rejection).into_response(),
        },
        Err(rejection) => return ApiError::from(rejection).into_response(),
    };

    // The tenant claim, which is what tells a staff token from every other
    // token this OP can mint. `None` is the `client_credentials` case and is
    // refused with the same 403 a wrong tenant gets: telling them apart would
    // say which of the two a caller had presented.
    if claims.merchant.as_deref() != Some(binding.merchant_id.as_str()) {
        tracing::warn!(
            subject = %claims.subject,
            carried_merchant_claim = claims.merchant.is_some(),
            "a validly signed dashboard-audience token does not carry this deployment's \
             merchant claim; refusing rather than serving a tenant nothing vouched for"
        );
        return ApiError::Forbidden.into_response();
    }
    if !claims.has_scope(&staff_binding.scope) {
        tracing::warn!(
            subject = %claims.subject,
            granted = ?claims.scope,
            required = %staff_binding.scope,
            "a /dash/v1 token does not carry the dashboard registration's scope; refusing"
        );
        return ApiError::Forbidden.into_response();
    }

    // THE STAFF ROW IS RE-READ HERE. `sub` names the `stf_…` the grant minted
    // for, and a disabled account must stop reading on its next request, not
    // when its token expires. Placed after the audience, tenant and scope
    // checks so that the read costs only a caller who has already proved a
    // validly signed, in-tenant, in-scope token: an unauthenticated caller
    // cannot make this surface touch Postgres. The row's OWN tenant is checked
    // too, and it is a second question from the merchant *claim*: the claim
    // says which tenant the token was minted for, this says which tenant the
    // person belongs to **now**.
    let staff = match state.repositories.find(&claims.subject).await {
        Ok(Some(staff)) if staff.is_active() && staff.merchant_id == binding.merchant_id => staff,
        Ok(_) => {
            tracing::warn!(
                subject = %claims.subject,
                "a /dash/v1 token names a staff member who is disabled, no longer exists, or no \
                 longer belongs to this deployment's merchant; refusing rather than serving rows \
                 to a revoked account for the rest of the token's TTL"
            );
            return ApiError::Forbidden.into_response();
        }
        Err(error) => {
            tracing::error!(%error, "reading the staff row behind a /dash/v1 token");
            return ApiError::from(error).into_response();
        }
    };

    let tenancy = match resolve_tenancy(&mut parts, state, &staff, binding).await {
        Ok(tenancy) => tenancy,
        Err(response) => return *response,
    };

    tracing::Span::current().record(AUTH_METHOD_FIELD, AUTH_METHOD_HUMAN_SESSION);
    parts.extensions.insert(MerchantScope::for_dashboard(
        tenancy.merchant_id().to_owned(),
    ));
    parts.extensions.insert(tenancy);
    parts.extensions.insert(claims);
    next.run(Request::from_parts(parts, body)).await
}

/// Which tenant this request may read (ADR-0018), for a member the caller
/// already proved is active and of the dashboard's merchant. **Both doors end
/// here**, so they cannot disagree about the rule.
///
/// A non-admin resolves to the *bound* one unconditionally, and never anything
/// the request said — the `?merchant_id=` parameter is not even PARSED unless
/// `staff.is_admin` is true, so nothing a non-admin writes can move it
/// (`a_non_admin_cannot_move_the_tenant_with_the_query_parameter` is the
/// end-to-end guard). It is read through `VpayQuery`, the decoder every other
/// query parameter goes through, so a malformed value fails the way a malformed
/// `limit` does.
async fn resolve_tenancy(
    parts: &mut axum::http::request::Parts,
    state: &DashScopeState,
    staff: &StaffRow,
    binding: &DashboardBinding,
) -> Result<DashboardTenancy, Box<Response>> {
    if !staff.is_admin {
        return Ok(DashboardTenancy::Bound(binding.merchant_id.clone()));
    }
    let requested =
        match crate::form::VpayQuery::<AdminMerchantOverride>::from_request_parts(parts, &()).await
        {
            Ok(crate::form::VpayQuery(over)) => over.merchant_id.filter(|value| !value.is_empty()),
            Err(rejection) => return Err(Box::new(rejection.into_response())),
        };
    match requested {
        // No override, or the admin named its own tenant: not a cross-tenant
        // read, and `is_cross_tenant` must answer `false`.
        None => Ok(DashboardTenancy::Bound(binding.merchant_id.clone())),
        Some(merchant_id) if merchant_id == binding.merchant_id => {
            Ok(DashboardTenancy::Bound(merchant_id))
        }
        Some(merchant_id) => {
            if !state.resource_config.is_known_merchant(&merchant_id) {
                tracing::warn!(
                    staff_id = %staff.id,
                    %merchant_id,
                    "an admin /dash/v1 request named a merchant this deployment does not \
                     serve; refusing rather than answering an empty page for a typo"
                );
                return Err(Box::new(
                    ApiError::invalid_param(
                        "merchant_id",
                        "Unknown merchant. An admin may read any merchant this deployment \
                         serves; this is not one of them.",
                    )
                    .into_response(),
                ));
            }
            // The one line ADR-0018's "blast radius" section asks for.
            tracing::info!(
                staff_id = %staff.id,
                home_merchant_id = %binding.merchant_id,
                %merchant_id,
                "an admin /dash/v1 request named a merchant other than its own; serving a \
                 cross-tenant read (ADR-0018)"
            );
            Ok(DashboardTenancy::ChosenByAdmin(merchant_id))
        }
    }
}
