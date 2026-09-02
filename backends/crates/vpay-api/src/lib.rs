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
//!
//! [`ApiError`] is this layer's Tier-2 composite error
//! ([ADR-0011](../../../docs/adr/0011-error-modelling.md)). Every failure
//! response in this crate is rendered by its `IntoResponse` — including the
//! 404 fallback below — so a handler returns `Result<_, ApiError>` and never
//! picks a status or writes a merchant-facing sentence. The one deliberate
//! exception is `/healthz`, which answers plain text for the reasons given
//! in [`error`]'s module docs.

use axum::extract::State;
use axum::http::{Method, Uri};
use axum::response::IntoResponse;
use axum::{Router, http::StatusCode, routing::get};
use serde_json::{Map, Value, json};
use vpay_db::PgPool;

pub mod error;
pub mod resource_auth;

pub use error::ApiError;

/// Stripe's error envelope, so SDK clients surface `.message` correctly.
///
/// Kept as the three-argument form because that is what an envelope without
/// a `param` is; see [`error_envelope_with_param`] for the fourth field and
/// why it is `Option` rather than always present. Nothing in production
/// calls this one — [`ApiError`]'s `IntoResponse` calls
/// [`error_envelope_with_param`] directly — and it stays because
/// `the_error_envelope_matches_stripes_shape` below pins the envelope's
/// shape through it, which is worth keeping independent of the classification
/// machinery that now decides what goes *into* it.
///
/// `pub(crate)`, like its four-argument sibling: ADR-0011 wants one renderer,
/// and visibility is what makes that structural instead of a convention a
/// handler can quietly break. `#[cfg(test)]` on top of that because the test
/// is now its only caller in any build — with no production caller left,
/// compiling it into the binary would be dead code the workspace's
/// `-D warnings` gate rightly refuses, and silencing that with an `allow`
/// would hide the very fact this comment is recording.
#[cfg(test)]
#[must_use = "the envelope is the response body"]
pub(crate) fn error_envelope(kind: &str, code: &str, message: &str) -> Value {
    error_envelope_with_param(kind, code, message, None)
}

/// [`error_envelope`] plus Stripe's optional `param`, naming the request
/// field a caller has to fix.
///
/// `param` is **omitted** rather than serialised as `null` when there is
/// none: Stripe omits it, an SDK testing `"param" in error` would be misled
/// by a null, and every response this crate has emitted so far had no such
/// key. Building the object by hand rather than mutating a `json!` literal
/// keeps that decision in one visible place.
///
/// **The one production envelope renderer**, called from [`ApiError`]'s
/// `IntoResponse` and nowhere else. `pub(crate)` on purpose: a handler in
/// another crate cannot reach it at all, so "handlers do not build envelopes"
/// is enforced by the module system rather than by review. The SDKs model
/// this shape from the wire (`sdks/rust`, `sdks/nodejs`), not from this
/// signature.
#[must_use = "the envelope is the response body"]
pub(crate) fn error_envelope_with_param(
    kind: &str,
    code: &str,
    message: &str,
    param: Option<&str>,
) -> Value {
    let mut error = Map::new();
    error.insert("type".to_owned(), Value::String(kind.to_owned()));
    error.insert("code".to_owned(), Value::String(code.to_owned()));
    error.insert("message".to_owned(), Value::String(message.to_owned()));
    if let Some(param) = param {
        error.insert("param".to_owned(), Value::String(param.to_owned()));
    }
    json!({ "error": Value::Object(error) })
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
///
/// Returns an [`ApiError`] rather than building an envelope here: ADR-0011
/// wants one renderer, and this handler used to be a second caller of
/// `error_envelope`. It is not one now, and since both envelope functions
/// became `pub(crate)` it could not be — `ApiError`'s `IntoResponse` is the
/// only production path to `error_envelope_with_param`. The response bytes
/// are unchanged — pinned verbatim by
/// `the_404_fallback_is_byte_for_byte_what_it_was_before_api_error`.
///
/// The method and path are captured for the log line only; the body
/// deliberately does not echo them back (see
/// [`ApiError::public_message`](vpay_core::Classify::public_message)).
async fn not_found(method: Method, uri: Uri) -> ApiError {
    ApiError::UnknownRoute {
        method: method.to_string(),
        path: uri.path().to_owned(),
    }
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
