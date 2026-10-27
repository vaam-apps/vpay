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
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cratestack_core::{CratestackError, VerifiedSigner};
use cratestack_cose::thumbprint::KID_LEN;
use cratestack_cose::{CoseAlg, CoseVerifierResolver, CoseVerifyKey};
use time::OffsetDateTime;
use vpay_db::Repositories;

use crate::dash::{AdminMerchantOverride, DashboardTenancy};
use crate::v1::MerchantScope;
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

/// What [`dash_scope`] needs: the repositories to re-read the key and the
/// member, and the configuration that names the one tenant.
#[derive(Clone)]
pub(crate) struct DashScopeState {
    pub(crate) repositories: Arc<dyn Repositories>,
    pub(crate) resource_config: Arc<ResourceConfig>,
}

/// The middleware. Mounted with `route_layer` directly around the `/dash/v1`
/// routes and the procedure routes, **inside** the envelope.
///
/// Refusals, all sealed because the signature verified:
///
/// | Situation | Answer |
/// |---|---|
/// | no `VerifiedSigner`, method `OPTIONS` | the router's own `405` (a CORS preflight the layer lets through unsigned; no console route answers it) |
/// | no `VerifiedSigner`, any other method | `500`: the layer is not in front of this route |
/// | the key was revoked or expired between the resolver's read and this one | `403` |
/// | the member is disabled, gone, or of another merchant than the dashboard's | `403`, one answer for all three |
/// | an administrator names an unknown merchant | `400` naming `merchant_id` |
pub(crate) async fn dash_scope(
    State(state): State<DashScopeState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();

    let Some(signer) = parts.extensions.get::<VerifiedSigner>() else {
        // The one request the layer lets through with no signature is a
        // bodiless `OPTIONS` (cratestack 0.16.0 passes it for a CORS
        // preflight). `route_layer` wraps a method router's own `405`
        // fallback, so it arrives here; no console route answers `OPTIONS`, so
        // what it reaches is axum's `405` with `Allow`. It is NOT the
        // misconfiguration below: an anonymous caller must not be able to
        // raise a page by asking.
        if parts.method == Method::OPTIONS {
            return next.run(Request::from_parts(parts, body)).await;
        }
        return ApiError::Internal(
            "a /dash/v1 request reached the console scope step with no VerifiedSigner: the \
             envelope layer is not mounted in front of this route"
                .to_owned(),
        )
        .into_response();
    };
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

    // Which tenant this request may read (ADR-0018). A non-admin resolves to
    // the *bound* one unconditionally, and never anything the request said —
    // the `?merchant_id=` parameter is not even PARSED unless `staff.is_admin`
    // is true, so nothing a non-admin writes can move it
    // (`a_non_admin_cannot_move_the_tenant_with_the_query_parameter` is the
    // end-to-end guard). It is read through `VpayQuery`, the decoder every
    // other query parameter goes through, so a malformed value fails the way a
    // malformed `limit` does.
    let tenancy = if staff.is_admin {
        let requested = match crate::form::VpayQuery::<AdminMerchantOverride>::from_request_parts(
            &mut parts,
            &(),
        )
        .await
        {
            Ok(crate::form::VpayQuery(over)) => over.merchant_id.filter(|value| !value.is_empty()),
            Err(rejection) => return rejection.into_response(),
        };
        match requested {
            // No override, or the admin named its own tenant: not a
            // cross-tenant read, and `is_cross_tenant` must answer `false`.
            None => DashboardTenancy::Bound(binding.merchant_id.clone()),
            Some(merchant_id) if merchant_id == binding.merchant_id => {
                DashboardTenancy::Bound(merchant_id)
            }
            Some(merchant_id) => {
                if !state.resource_config.is_known_merchant(&merchant_id) {
                    tracing::warn!(
                        staff_id = %staff.id,
                        %merchant_id,
                        "an admin /dash/v1 request named a merchant this deployment does not \
                         serve; refusing rather than answering an empty page for a typo"
                    );
                    return ApiError::invalid_param(
                        "merchant_id",
                        "Unknown merchant. An admin may read any merchant this deployment \
                         serves; this is not one of them.",
                    )
                    .into_response();
                }
                // The one line ADR-0018's "blast radius" section asks for.
                tracing::info!(
                    staff_id = %staff.id,
                    home_merchant_id = %binding.merchant_id,
                    %merchant_id,
                    "an admin /dash/v1 request named a merchant other than its own; serving a \
                     cross-tenant read (ADR-0018)"
                );
                DashboardTenancy::ChosenByAdmin(merchant_id)
            }
        }
    } else {
        DashboardTenancy::Bound(binding.merchant_id.clone())
    };

    parts.extensions.insert(MerchantScope::for_dashboard(
        tenancy.merchant_id().to_owned(),
    ));
    parts.extensions.insert(tenancy);
    next.run(Request::from_parts(parts, body)).await
}
