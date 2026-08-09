//! The Stripe-shaped HTTP surface.
//!
//! STATUS: only `/healthz` and the Stripe-shaped 404 envelope are implemented.
//! No `/v1/*` route exists yet. See `docs/STATUS.md` — this file must never
//! grow a route that returns fabricated data.

use axum::{Json, Router, http::StatusCode, routing::get};
use serde_json::{Value, json};

/// Stripe's error envelope, so SDK clients surface `.message` correctly.
#[must_use = "the envelope is the response body"]
pub fn error_envelope(kind: &str, code: &str, message: &str) -> Value {
    json!({ "error": { "type": kind, "code": code, "message": message } })
}

async fn healthz() -> &'static str {
    "ok"
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

pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(not_found)
}

#[cfg(test)]
mod tests {
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
}
