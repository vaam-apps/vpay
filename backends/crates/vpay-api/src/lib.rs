//! The Stripe-shaped HTTP surface.
//!
//! STATUS: `/healthz` and the Stripe-shaped 404 envelope are implemented.
//! No `/v1/*` route exists yet. See `docs/status.md` — this file must never
//! grow a route that returns fabricated data. A real database check (below)
//! is the opposite of fabricated data, so it stays.
//!
//! [`resource_auth`] adds bearer-token validation for `/v1` and `/dash/v1`
//! (OP-3) — a real `JwtValidator`/extractor pair, unit-tested against a real
//! JWKS server, but **not mounted onto any route here**. Nothing in this
//! file's `router()` changes as part of that: mounting `/v1`/`/dash/v1` is
//! later work that needs the OP assembled first.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Json, Router, http::StatusCode, routing::get};
use serde_json::{Value, json};
use vpay_db::PgPool;

pub mod resource_auth;

/// Stripe's error envelope, so SDK clients surface `.message` correctly.
#[must_use = "the envelope is the response body"]
pub fn error_envelope(kind: &str, code: &str, message: &str) -> Value {
    json!({ "error": { "type": kind, "code": code, "message": message } })
}

/// Shared state for every route in this router. Just the pool today; grows
/// as real routes land.
#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

/// `/healthz`: reflects whether the database is actually reachable right
/// now, rather than a static `"ok"`.
///
/// Single combined liveness+readiness endpoint, deliberately not split into
/// `/healthz` (process alive) and `/readyz` (DB reachable): nothing in this
/// repository defines a Kubernetes liveness vs. readiness probe today — no
/// manifest, no `Dockerfile HEALTHCHECK` (the runtime image is `FROM
/// scratch` and has no shell to run one — see `backends/Dockerfile`), no
/// `compose.yml` healthcheck on `vpay-server` itself. Inventing a real
/// liveness/readiness split ahead of an actual orchestration consumer that
/// would treat them differently would be exactly the kind of feature that
/// only *looks* more finished than it is (`CLAUDE.md`: "never make the repo
/// look more finished than it is") — a `/readyz` nobody polls proves
/// nothing. When real k8s manifests land, split this: a liveness probe
/// should stay a static `"ok"` (so a transient database blip does not cause
/// a pod restart that cannot fix a database outage), and a new readiness
/// probe should carry this DB check instead.
async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match vpay_db::check_connection(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(error) => {
            tracing::error!(%error, "healthz: database unreachable");
            (StatusCode::SERVICE_UNAVAILABLE, "database unreachable")
        }
    }
}

/// Honest 404 for every unimplemented route, naming vpay rather than pretending
/// to be Stripe.
async fn not_found() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(error_envelope(
            "invalid_request_error",
            "unknown_route",
            "Unrecognized request URL. vpay implements a subset of the Stripe API; see docs/api.",
        )),
    )
}

/// Builds the router. `pool` is required, not optional: every route this
/// binary serves — starting with `/healthz` — now depends on the database
/// being real, so a router without one would be a router that cannot tell
/// the truth about its own health.
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(not_found)
        .with_state(AppState { pool })
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    use super::*;

    #[test]
    fn the_error_envelope_matches_stripes_shape() {
        let v = error_envelope("invalid_request_error", "x", "y");
        let err = v.get("error").expect("error key");
        assert_eq!(
            err.get("type").and_then(Value::as_str),
            Some("invalid_request_error")
        );
        assert_eq!(err.get("code").and_then(Value::as_str), Some("x"));
        assert_eq!(err.get("message").and_then(Value::as_str), Some("y"));
    }

    /// `connect_lazy` parses the URL and builds a pool without performing any
    /// I/O — no real Postgres, no testcontainer, no fake data returned by
    /// the route under test. This proves the router wires state through and
    /// the fallback still 404s; it does *not* claim to prove `/healthz`
    /// against a live database — that is `vpay-db`'s own
    /// `tests/postgres.rs`, against a real `postgres:16-alpine` container,
    /// and this crate does not duplicate that infrastructure.
    fn lazy_pool() -> PgPool {
        PgPool::connect_lazy("postgres://vpay:vpay@localhost:5432/vpay")
            .expect("connect_lazy performs no I/O and only fails on a malformed URL")
    }

    #[tokio::test]
    async fn unknown_routes_still_get_the_honest_404() {
        let app = router(lazy_pool());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/payment_intents")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
