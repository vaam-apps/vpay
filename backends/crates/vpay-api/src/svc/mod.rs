//! `/svc/v1` — the routes a registered **service** calls, never a merchant and
//! never a person (VP-6a,
//! [ADR-0036](../../../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).
//!
//! Two routes, both `console_pairing`'s:
//!
//! | Route | Does |
//! |---|---|
//! | `POST /svc/v1/console_keys` | pairs a console key to the person vaam names |
//! | `DELETE /svc/v1/console_keys/{thumbprint}` | revokes one |
//!
//! ```text
//!   vaam ──COSE_Sign1(JSON)──▶ EnvelopeLayer ──▶ service_scope ──▶ handler
//!         (an Ed25519 key in         │               │                │
//!          service_clients)          │               └ capability?    └ needs the
//!                                    └ ServiceKeyResolver only          PairingAuthority
//!                                      (a merchant's key is no key)     the scope minted
//! ```
//!
//! # Who may call, and why a merchant never can
//!
//! The layer resolves a signature's key against the **service registry**
//! (`service_clients[].cose_keys`) and nothing else, so a merchant's key is not a
//! key here: the same coarse, unsigned `401` an unknown key gets. And a service
//! key reaches a handler only through [`crate::envelope::service`]'s scope, which
//! checks the capability **this route's row** in [`SVC_ROUTES`] names; the handler
//! then demands a [`PairingAuthority`], a type only that step can build. A
//! capability is a field of the YAML and nothing else — no route here, or
//! anywhere, can grant one.
//!
//! The wire is JSON in and out (`application/json`): a service-to-service call,
//! not a Stripe-shaped one. Errors after the layer are still the Stripe-shaped
//! envelope, sealed.

use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, delete, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use vpay_config::ServiceCapability;
use vpay_db::ConsoleKeyRow;

use crate::console_keys::{self, PairRequest};
use crate::{ApiError, AppState};

/// The path [`crate::router`] mounts this surface at.
pub const SVC_NEST: &str = vpay_wire::SVC_MOUNT;

/// One mounted `/svc/v1` route, and the capability a service needs to call it.
///
/// The same shape as [`crate::V1Route`] and [`crate::DashRoute`], with the one
/// addition this surface exists for: `capability` is in the **table**, so the
/// scope step that enforces it reads the same row the router was built from.
#[derive(Debug)]
pub struct SvcRoute {
    /// The axum path pattern, relative to [`SVC_NEST`].
    pub path: &'static str,
    /// Every HTTP method this path answers, upper-case.
    pub methods: &'static [&'static str],
    /// What a service must hold to call it.
    pub capability: ServiceCapability,
    /// Builds the handlers.
    pub(crate) mount: fn() -> MethodRouter<AppState>,
}

/// Every route mounted under `/svc/v1`, and the only place they are listed.
pub const SVC_ROUTES: &[SvcRoute] = &[
    SvcRoute {
        path: "/console_keys",
        methods: &["POST"],
        capability: ServiceCapability::ConsolePairing,
        mount: || post(pair),
    },
    SvcRoute {
        path: "/console_keys/{thumbprint}",
        methods: &["DELETE"],
        capability: ServiceCapability::ConsolePairing,
        mount: || delete(revoke),
    },
];

/// The `/svc/v1` router, folded from [`SVC_ROUTES`].
pub(crate) fn routes() -> Router<AppState> {
    SVC_ROUTES
        .iter()
        .fold(Router::new(), |router, route| {
            router.route(route.path, (route.mount)())
        })
        // Inside the nest, for `crate::v1::routes`' reason.
        .fallback(crate::not_found)
}

/// The capability row for a matched route and method, or `None`.
///
/// `matched` is axum's matched template **with** the mount (`/svc/v1/console_keys`).
#[must_use]
pub(crate) fn capability_for(matched: &str, method: &str) -> Option<ServiceCapability> {
    let template = matched.strip_prefix(SVC_NEST)?;
    SVC_ROUTES
        .iter()
        .find(|route| route.path == template && route.methods.contains(&method))
        .map(|route| route.capability)
}

/// Proof that the request was signed by a service holding `console_pairing`.
///
/// A private field and no public constructor, `MerchantScope`'s device: the
/// scope step ([`crate::envelope::service`]) is the only code that builds one,
/// after it has checked the capability against the service registry. A handler
/// that asks for it and runs without that step gets a `500`, never a pairing.
#[derive(Debug, Clone)]
pub struct PairingAuthority {
    client_id: String,
}

impl PairingAuthority {
    /// Minted by the scope step for the service `client_id`, and by nothing else.
    pub(crate) fn granted_to(client_id: String) -> Self {
        Self { client_id }
    }

    /// The service that holds it, for the log.
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
}

impl<S> FromRequestParts<S> for PairingAuthority
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Self>().cloned().ok_or_else(|| {
            ApiError::Internal(
                "a /svc/v1 handler ran with no PairingAuthority on the request: the service \
                 scope step is not mounted in front of this route"
                    .to_owned(),
            )
        })
    }
}

/// The body of `POST /svc/v1/console_keys`.
///
/// **`deny_unknown_fields`, and that is a security property**: there is no
/// `is_admin`, `role` or `merchant_id` here, and a body that sends one is a
/// `400` rather than a field that is silently ignored — so nothing a service
/// sends can be mistaken for a way to create or upgrade an administrator, or to
/// name another tenant.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct PairBody {
    /// The person's id at vaam.
    pub vaam_user_id: String,
    /// What the console greets them by.
    #[serde(default)]
    pub display_name: Option<String>,
    /// The console key, SEC1 uncompressed P-256, unpadded base64url.
    pub public_key: String,
    /// ECDSA-P-256-SHA-256 over `"vpay.console-pair.v1\n" + public_key + "\n" +
    /// issued_at`, `r || s`, unpadded base64url.
    pub possession_proof: String,
    /// When the console signed the proof, unix seconds.
    pub issued_at: i64,
    /// A name for the key in logs.
    #[serde(default)]
    pub label: Option<String>,
}

/// A console key on the wire: never the point itself, only what a service and an
/// operator need to name it and know its life.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ConsoleKeyObject {
    /// Always `console_key`.
    pub object: &'static str,
    /// The thumbprint: the key's name everywhere.
    pub thumbprint: String,
    /// The member it signs as.
    pub staff_member_id: String,
    /// The algorithm, `esp256`.
    pub alg: String,
    /// The operator's name for the key.
    pub label: Option<String>,
    /// When it was paired, unix seconds.
    pub paired_at: i64,
    /// When it stops authenticating, unix seconds: `paired_at` plus 30 days.
    pub expires_at: i64,
    /// When it was revoked, unix seconds, or `null`.
    pub revoked_at: Option<i64>,
}

impl From<ConsoleKeyRow> for ConsoleKeyObject {
    fn from(row: ConsoleKeyRow) -> Self {
        Self {
            object: "console_key",
            thumbprint: row.thumbprint,
            staff_member_id: row.staff_member_id,
            alg: row.alg,
            label: row.label,
            paired_at: row.paired_at.unix_timestamp(),
            expires_at: row.expires_at.unix_timestamp(),
            revoked_at: row.revoked_at.map(OffsetDateTime::unix_timestamp),
        }
    }
}

/// `POST /svc/v1/console_keys` — pair a console key to a person vaam vouches for.
///
/// `201` when this call stored the key, `200` when the same live key of the same
/// person was already there.
async fn pair(
    authority: PairingAuthority,
    State(state): State<AppState>,
    body: Result<Json<PairBody>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(body) = body?;
    // The tenant is the configured dashboard's, and nothing else: the body has
    // no field that could name another.
    let tenant = state
        .resource_config
        .dashboard()
        .ok_or_else(|| ApiError::UnknownRoute {
            method: "POST".to_owned(),
            path: format!("{SVC_NEST}/console_keys"),
        })?
        .merchant_id
        .clone();
    let paired = console_keys::pair_through_vaam(
        &state.repositories,
        &tenant,
        PairRequest {
            vaam_user_id: body.vaam_user_id,
            display_name: body.display_name,
            public_key: body.public_key,
            possession_proof: body.possession_proof,
            issued_at: body.issued_at,
            label: body.label,
        },
        OffsetDateTime::now_utc(),
    )
    .await?;
    tracing::info!(
        service = authority.client_id(),
        thumbprint = %paired.key.thumbprint,
        created = paired.created,
        "a console key was paired through a service"
    );
    let status = if paired.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(ConsoleKeyObject::from(paired.key))).into_response())
}

/// `DELETE /svc/v1/console_keys/{thumbprint}` — revoke a console key. Effective
/// on the key's next request; idempotent.
async fn revoke(
    authority: PairingAuthority,
    State(state): State<AppState>,
    Path(thumbprint): Path<String>,
) -> Result<Json<ConsoleKeyObject>, ApiError> {
    let row = console_keys::revoke(
        state.repositories.as_ref(),
        &thumbprint,
        OffsetDateTime::now_utc(),
    )
    .await?;
    tracing::info!(
        service = authority.client_id(),
        thumbprint = %row.thumbprint,
        "a console key was revoked through a service"
    );
    Ok(Json(ConsoleKeyObject::from(row)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A route with no capability row is a route the scope step cannot guard;
    /// every row names one, and the lookup finds exactly the rows.
    #[test]
    fn every_route_names_its_capability_and_the_lookup_finds_it() {
        for route in SVC_ROUTES {
            for method in route.methods {
                assert_eq!(
                    capability_for(&format!("{SVC_NEST}{}", route.path), method),
                    Some(route.capability)
                );
            }
        }
        assert_eq!(capability_for("/svc/v1/console_keys", "GET"), None);
        assert_eq!(capability_for("/v1/console_keys", "POST"), None);
        assert_eq!(capability_for("/svc/v1/nothing", "POST"), None);
    }

    /// `/svc/v1`'s table and `vpay-wire`'s agree in both directions: a route
    /// with no contract would be a `500` for every signed call to it, and a
    /// contract with no route is a row nothing answers.
    #[test]
    fn the_route_table_and_the_wire_table_list_the_same_operations() {
        let mounted: std::collections::BTreeSet<(String, String)> = SVC_ROUTES
            .iter()
            .flat_map(|route| {
                route
                    .methods
                    .iter()
                    .map(|method| ((*method).to_owned(), route.path.to_owned()))
            })
            .collect();
        let contracted: std::collections::BTreeSet<(String, String)> = vpay_wire::SVC_OPERATIONS
            .iter()
            .map(|operation| (operation.method.to_owned(), operation.path.to_owned()))
            .collect();
        assert_eq!(mounted, contracted);
    }

    #[test]
    fn the_pair_body_refuses_a_field_that_would_name_a_role_or_a_tenant() {
        let base = r#"{"vaam_user_id":"u","public_key":"k","possession_proof":"p","issued_at":1}"#;
        assert!(serde_json::from_str::<PairBody>(base).is_ok());
        for extra in ["is_admin", "role", "merchant_id", "staff_member_id"] {
            let with = base.replace('}', &format!(r#","{extra}":true}}"#));
            assert!(
                serde_json::from_str::<PairBody>(&with).is_err(),
                "{extra} must be refused, not ignored"
            );
        }
    }
}
