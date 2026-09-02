//! The `Idempotency-Key` header and the request fingerprint that goes with
//! it.
//!
//! Stripe's semantics, and Step 2's D7 makes the header **required** on every
//! `POST` under `/v1` rather than optional-with-a-generated-fallback. That is
//! a deliberate divergence from Stripe, and the reason is that the fallback is
//! worthless: a key this server invents is different on every attempt, so a
//! merchant whose `POST /v1/payment_intents` times out and is retried would
//! create two payments and find out from their customer. Refusing the request
//! with a `400` naming `idempotency_key` is a bug the merchant finds in
//! development, in one request, instead. Both SDKs already send one on every
//! `POST` (`docs/flows/merchant-auth.md`'s header table), so the requirement
//! costs a correct integration nothing.
//!
//! # What the key alone cannot decide
//!
//! A key identifies an *attempt*; it does not say whether the attempt is the
//! same one. Two different bodies under one key must not answer the first
//! body's object — that is how a merchant "retrying" a 5,000 XAF charge gets
//! back the receipt for a 50,000 XAF one. So a key is always stored with
//! [`request_hash`] of the request that first used it, and a replay whose hash
//! differs is [`ApiError::idempotency_key_reused`] (`400`,
//! `idempotency_error`) rather than a replayed body. The storage and the
//! comparison live in `vpay_db::idempotency`; this module owns the header and
//! the fingerprint.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sha2::{Digest as _, Sha256};

use crate::ApiError;

/// The header, spelled once. Stripe's name, lower-cased because `http`'s
/// `HeaderMap` lookups are case-insensitive but the constant should not
/// suggest otherwise.
const HEADER: &str = "idempotency-key";

/// The longest key accepted, in bytes.
///
/// Stripe documents 255 characters. Bytes rather than characters here because
/// a header value is bytes on the wire and the check must be on what was
/// actually sent — and because the ASCII-printable rule below means the two
/// counts are the same for every key this accepts anyway.
const MAX_KEY_BYTES: usize = 255;

/// The `param` every rejection from this module names, so an SDK points at the
/// header rather than at a body field.
const PARAM: &str = "idempotency_key";

/// A validated `Idempotency-Key`.
///
/// Newtype rather than a bare `String` so a handler cannot pass a merchant id,
/// a path segment or an unvalidated header where a key belongs: the only way
/// to obtain one is to have extracted it, which is also the only place the
/// length and character rules are applied.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// The key as the merchant sent it — for storage and comparison only.
    ///
    /// Never render this into a response: a key can carry an order number, a
    /// customer reference, occasionally a name. [`ApiError`] truncates it to a
    /// hint on the one path that echoes anything at all.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validates a raw header value.
    ///
    /// Separate from the extractor so the rules are testable without building
    /// a request, and so `vpay-worker` (which will replay stored requests)
    /// could reuse them.
    ///
    /// # Errors
    /// [`ApiError::InvalidParam`] with `param: "idempotency_key"` — absent or
    /// blank, longer than `MAX_KEY_BYTES`, or holding a byte outside
    /// printable US-ASCII.
    pub fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        let Some(value) = value else {
            return Err(missing());
        };
        // A key of spaces names nothing, and a client that sent one almost
        // certainly meant to interpolate a variable that was empty. Treated as
        // absent rather than accepted, because accepting it would make every
        // such request collide with every other one.
        if value.trim().is_empty() {
            return Err(missing());
        }
        if value.len() > MAX_KEY_BYTES {
            return Err(ApiError::invalid_param(
                PARAM,
                "The Idempotency-Key header must be at most 255 bytes.",
            ));
        }
        // Printable US-ASCII only. `http` already refuses most control bytes
        // in a header value, but not DEL or every byte above 0x7f — and a key
        // is a primary key in Postgres, a log field and a support-ticket
        // quotation. Restricting it here means none of those has to think
        // about encoding.
        if !value.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
            return Err(ApiError::invalid_param(
                PARAM,
                "The Idempotency-Key header must contain only printable US-ASCII characters.",
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

/// The rejection for "no usable key at all". One function so the sentence
/// D7 fixes is written exactly once.
fn missing() -> ApiError {
    ApiError::invalid_param(
        PARAM,
        "An Idempotency-Key header is required on every POST to /v1.",
    )
}

impl<S> FromRequestParts<S> for IdempotencyKey
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // A header whose bytes are not UTF-8 is reported as *absent* rather
        // than as its own case: the merchant has to fix it the same way
        // either way, and the alternative message would have to describe our
        // encoding rules to someone who has not read them.
        Self::parse(parts.headers.get(HEADER).and_then(|v| v.to_str().ok()))
    }
}

/// The fingerprint stored beside an idempotency key: SHA-256 over the request
/// method, path and raw body.
///
/// **Length-prefixed, not concatenated.** `POST` + `/v1/payment_intents` and
/// `POST/v1` + `/payment_intents` are different requests that a plain
/// concatenation would hash identically; a merchant could then replay one
/// endpoint's key on another and be handed the first's stored response. Each
/// field's length goes in ahead of it, so exactly one input produces any given
/// digest.
///
/// The **raw** body, before decoding: two bodies that differ only in
/// percent-escaping (`%2B` vs a literal `+`) really are different requests to
/// this API, and hashing the decoded form would let one masquerade as the
/// other. It also means this can be computed before anything is parsed, which
/// is what lets a replay be answered without the parse running at all.
///
/// The merchant id is **not** hashed: it is part of the storage key
/// (`PRIMARY KEY (merchant_id, idempotency_key)`, migration 0015), so two
/// merchants using the same key are already separate rows and a hash
/// collision across tenants is not reachable.
#[must_use]
pub fn request_hash(method: &str, path: &str, body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for field in [method.as_bytes(), path.as_bytes(), body] {
        // `u64`, big-endian, so the framing is the same on every platform —
        // this digest is written to a database and compared against on a
        // later request, possibly by a different process.
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use serde_json::Value;
    use tower::ServiceExt as _;

    use super::*;

    async fn handler(key: IdempotencyKey) -> String {
        key.as_str().to_owned()
    }

    /// A router carrying the same state a real `/v1` route does, so the
    /// extractor is exercised the way it will be mounted rather than against a
    /// bare `()` state.
    fn app() -> Router {
        Router::new().route("/v1/payment_intents", post(handler))
    }

    async fn send(request: Request<Body>) -> (StatusCode, String) {
        let response = app()
            .oneshot(request)
            .await
            .expect("the router does not fail to serve");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn post_with(key: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method("POST").uri("/v1/payment_intents");
        if let Some(key) = key {
            builder = builder.header("Idempotency-Key", key);
        }
        builder.body(Body::empty()).expect("a valid request")
    }

    #[tokio::test]
    async fn a_key_reaches_the_handler_verbatim() {
        let (status, body) = send(post_with(Some("order_1234_attempt_1"))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "order_1234_attempt_1");
    }

    /// D7, as the thing a merchant actually sees. The envelope is asserted
    /// field by field because an SDK reads `param` to point at the header.
    #[tokio::test]
    async fn a_post_without_the_header_is_the_documented_400() {
        let (status, body) = send(post_with(None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let envelope: Value = serde_json::from_str(&body).expect("a JSON envelope");
        let error = envelope.get("error").expect("an error object");
        assert_eq!(
            error.get("type").and_then(Value::as_str),
            Some("invalid_request_error")
        );
        assert_eq!(
            error.get("param").and_then(Value::as_str),
            Some("idempotency_key")
        );
        assert_eq!(
            error.get("message").and_then(Value::as_str),
            Some("An Idempotency-Key header is required on every POST to /v1.")
        );
    }

    #[tokio::test]
    async fn an_empty_or_blank_key_is_the_same_as_none() {
        for key in ["", "   ", "\t"] {
            let (status, body) = send(post_with(Some(key))).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{key:?}");
            assert!(
                body.contains("is required on every POST"),
                "{key:?}: {body}"
            );
        }
    }

    #[tokio::test]
    async fn a_key_at_the_bound_is_accepted_and_one_byte_over_is_not() {
        let at_bound = "k".repeat(MAX_KEY_BYTES);
        let (status, body) = send(post_with(Some(&at_bound))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, at_bound);

        let over = "k".repeat(MAX_KEY_BYTES + 1);
        let (status, body) = send(post_with(Some(&over))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("255 bytes"), "{body}");
    }

    #[test]
    fn only_printable_ascii_is_a_key() {
        for good in ["k", "order-1234", "a b", "~!@#$%^&*()_+={}[]|;:'\",.<>/?"] {
            assert!(IdempotencyKey::parse(Some(good)).is_ok(), "{good:?}");
        }
        for bad in ["café", "key\u{7f}", "key\u{0}", "clé-1234"] {
            let error = IdempotencyKey::parse(Some(bad))
                .err()
                .unwrap_or_else(|| panic!("{bad:?} must be refused"));
            assert_eq!(error.param(), Some(PARAM), "{bad:?}");
        }
    }

    // --- the fingerprint ---

    #[test]
    fn the_same_request_hashes_the_same_and_a_changed_body_does_not() {
        let a = request_hash("POST", "/v1/payment_intents", b"amount=5000&currency=xaf");
        let b = request_hash("POST", "/v1/payment_intents", b"amount=5000&currency=xaf");
        assert_eq!(a, b, "the same request must hash to the same digest");

        let changed = request_hash("POST", "/v1/payment_intents", b"amount=50000&currency=xaf");
        assert_ne!(
            a, changed,
            "an amount a merchant did not mean to change must not replay the first response"
        );
    }

    /// The reason each field is length-prefixed. Without the framing these two
    /// requests hash identically, and a key spent on one endpoint would replay
    /// on another.
    #[test]
    fn the_method_path_and_body_cannot_be_shifted_across_each_other() {
        assert_ne!(
            request_hash("POST", "/v1/payment_intents", b""),
            request_hash("POST/v1", "/payment_intents", b"")
        );
        assert_ne!(
            request_hash("POST", "/v1/payment_intents", b"a=1"),
            request_hash("POST", "/v1/payment_intents/a=1", b"")
        );
        assert_ne!(
            request_hash("POST", "/v1/payment_intents/pi_1/confirm", b"a=1"),
            request_hash("POST", "/v1/payment_intents/pi_2/confirm", b"a=1")
        );
    }

    /// The digest is stored in Postgres (`request_hash BYTEA CHECK
    /// (octet_length(request_hash) = 32)`, migration 0015) and compared on a
    /// later request, so its width and its stability across processes are part
    /// of the schema, not an implementation detail. The vector is
    /// SHA-256("\x00..\x04POST\x00..\x03/v1\x00..\x03a=1") — recomputed
    /// independently below rather than pasted from this function's own output.
    #[test]
    fn the_digest_is_thirty_two_bytes_of_sha256_over_the_framed_fields() {
        let hash = request_hash("POST", "/v1", b"a=1");
        assert_eq!(hash.len(), 32);

        let mut expected = Sha256::new();
        expected.update(4u64.to_be_bytes());
        expected.update(b"POST");
        expected.update(3u64.to_be_bytes());
        expected.update(b"/v1");
        expected.update(3u64.to_be_bytes());
        expected.update(b"a=1");
        let expected: [u8; 32] = expected.finalize().into();
        assert_eq!(hash, expected);

        // And it is not the digest of the fields simply run together, which is
        // what an implementation without the framing would produce.
        let mut unframed = Sha256::new();
        unframed.update(b"POST/v1a=1");
        let unframed: [u8; 32] = unframed.finalize().into();
        assert_ne!(hash, unframed);
    }

    /// Escaping is part of the request: `%2B` and `+` decode differently in
    /// this API (`crate::form`), so they must not share a fingerprint.
    #[test]
    fn two_bodies_that_differ_only_in_escaping_are_two_requests() {
        assert_ne!(
            request_hash("POST", "/v1/x", b"msisdn=%2B237670000000"),
            request_hash("POST", "/v1/x", b"msisdn=+237670000000")
        );
    }
}
