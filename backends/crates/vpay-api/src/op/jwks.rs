//! vpay's own `/jwks.json`: every signing key currently inside its rotation
//! window, read from the database rather than from this process's memory.
//!
//! # Why not `authkestra_axum::axum_jwks_handler`
//!
//! Authkestra ships a JWKS handler, and vpay does not use it. That handler
//! serves `token_manager.public_jwk()` — **the one key the process it runs in
//! happens to hold**, wrapped in a `keys` array of length one. That is
//! correct for a deployment whose key never changes, and wrong for this one
//! in two ways that both break token verification:
//!
//! 1. It cannot serve a rotation window. The moment a new key is deployed,
//!    the previous key vanishes from the JWKS — but tokens it signed are
//!    still inside their TTL and still being presented. Every one of them
//!    fails with an unknown `kid`. Publishing the retired key until its
//!    `expires_at` ([`keys::ROTATION_OVERLAP`](super::keys::ROTATION_OVERLAP))
//!    is the entire purpose of the `oauth_signing_keys` table.
//! 2. It answers from process-local state during a rolling deploy. With N
//!    replicas mid-rollout, a JWKS fetch is load-balanced to *some* pod, so
//!    the answer depends on which one — old key or new key, whichever
//!    replica took the request. Reading from Postgres makes every replica
//!    answer the same thing.
//!
//! So the document is assembled from [`vpay_db::publishable_signing_keys`]
//! (`WHERE active OR expires_at > now()`), and this module is the only place
//! that shape is built.
//!
//! # Where it is mounted
//!
//! [`jwks_handler`] is served at `GET /v1/oauth/jwks.json`, unauthenticated,
//! by [`crate::router`] — see the route table in that function's docs for
//! why this path (and the rest of the `/v1/oauth` subtree) sits outside the
//! merchant bearer-token boundary. This module only builds the document; the
//! path and the auth decision belong to the route table, so change them
//! there.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use serde_json::{Value, json};
use vpay_db::{PgPool, SigningKey};

use crate::error::ApiError;

/// The `Cache-Control: max-age` this document is served with, in seconds.
///
/// Five minutes, and the reasoning is about *who* is caching. vpay's own
/// resource-server validator does not depend on this header at all: it holds
/// its own [`JwksCache`](authkestra_resource::jwt::JwksCache) on a refresh
/// interval it chooses, and it refetches immediately on an unrecognised
/// `kid`, so a rotation reaches it without waiting for any TTL to lapse
/// ([`crate::resource_auth`]). What this header bounds is everything vpay
/// does not control — a CDN, an ingress cache, a merchant's HTTP client —
/// and there the tradeoff is: long enough that a public, unauthenticated
/// endpoint is not a per-request database read for anyone who points a
/// load generator at it, short enough that a newly rotated key is picked up
/// in minutes rather than hours.
///
/// It is safe precisely *because* of the rotation overlap: a stale-by-300 s
/// copy of this document still contains the retired key it was published
/// with, and the new key becomes discoverable long before the old one stops
/// being published 24 hours later. `public` rather than `private`: this
/// document is public by definition — it is a set of public keys, served
/// without authentication, and identical for every caller.
pub const JWKS_CACHE_MAX_AGE: u64 = 300;

/// Builds the `{"keys": [...]}` document from database rows.
///
/// Pure and separately testable on purpose: the interesting decisions here
/// are about *which* keys appear and in what shape, and none of them need a
/// database or an HTTP request to exercise.
///
/// Order follows the input, which [`vpay_db::publishable_signing_keys`]
/// orders by `created_at` — so the document is stable across requests and a
/// diff between two fetches means something actually changed. Order carries
/// no meaning to a verifier: consumers select by `kid`, and vpay's own
/// validator is built with `require_kid(true)` specifically so that no
/// verifier ever falls back to "the first key in the list".
///
/// **A JWK with no `kid` member is skipped, with a warning**, rather than
/// published. The validator that consumes this document
/// ([`crate::resource_auth::JwtValidator`]) looks keys up by `kid` and
/// rejects a token that has none, so a JWK without one cannot ever be
/// selected — publishing it would add an entry that no verifier can use
/// while making the document look healthier than it is. Skipping is not a
/// silent fix: it is logged at `warn` with the row's own `kid` column, which
/// is enough for an operator to find the row, because a row whose stored JWK
/// disagrees with its key column means something wrote this table by hand.
#[must_use]
pub fn jwks_document(keys: &[SigningKey]) -> Value {
    let published: Vec<Value> = keys
        .iter()
        .filter(|key| is_publishable(key))
        .map(|key| key.public_jwk.clone())
        .collect();

    json!({ "keys": published })
}

/// Whether one row's stored JWK can actually serve a verifier — see
/// [`jwks_document`] for why a missing `kid` is a skip rather than a
/// pass-through.
fn is_publishable(key: &SigningKey) -> bool {
    let Some(jwk) = key.public_jwk.as_object() else {
        tracing::warn!(
            kid = %key.kid,
            "oauth_signing_keys row's public_jwk is not a JSON object; excluded from /jwks.json"
        );
        return false;
    };

    let Some(jwk_kid) = jwk.get("kid").and_then(Value::as_str) else {
        tracing::warn!(
            kid = %key.kid,
            "oauth_signing_keys row's public_jwk has no `kid` member; excluded from /jwks.json \
             because no verifier could ever select it"
        );
        return false;
    };

    // Published anyway: the JWK is what a verifier matches against, so the
    // document stays usable. But the two disagreeing means the row was not
    // written by `LoadedSigningKey::ensure_active_in_database`, which
    // derives both from the same thumbprint — worth an operator's attention.
    if jwk_kid != key.kid {
        tracing::warn!(
            row_kid = %key.kid,
            jwk_kid = %jwk_kid,
            "oauth_signing_keys row's `kid` column disagrees with its published JWK's `kid`"
        );
    }

    true
}

/// `GET /v1/oauth/jwks.json` — every key inside its rotation window.
///
/// Unauthenticated by definition: this is how a verifier that has never
/// spoken to vpay before learns the public keys, so requiring a credential
/// would be circular.
///
/// Takes `State<PgPool>` rather than the assembler's whole state type, so
/// this handler stays independent of how the router is put together; the
/// assembler supplies `impl FromRef<AppState> for PgPool`.
///
/// # Errors
///
/// Returns [`ApiError::Db`] if the keys cannot be read. Deliberately *not*
/// an empty document on failure: an empty `{"keys":[]}` is a valid JWKS
/// meaning "this issuer has no keys", which every verifier would cache and
/// act on. A 503 says "ask again", which is the truth.
pub async fn jwks_handler(State(pool): State<PgPool>) -> Result<impl IntoResponse, ApiError> {
    let keys = vpay_db::publishable_signing_keys(&pool).await?;
    let document = jwks_document(&keys);

    if keys.is_empty() {
        // Not an error: the query succeeded and this is genuinely what the
        // database says. It does mean no token this deployment ever issued
        // can be verified, so it is worth being loud about.
        tracing::warn!(
            "oauth_signing_keys has no publishable key; /jwks.json is empty and no token issued \
             by this deployment can be verified"
        );
    }

    Ok((cache_control(), axum::Json(document)))
}

/// The one construction of this document's `Cache-Control` header, so the
/// header a caller receives and the header the tests assert on cannot be two
/// different strings. Derived from [`JWKS_CACHE_MAX_AGE`] rather than
/// spelled out, so the constant and its doc comment stay authoritative.
fn cache_control() -> [(header::HeaderName, String); 1] {
    [(
        header::CACHE_CONTROL,
        format!("public, max-age={JWKS_CACHE_MAX_AGE}"),
    )]
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;
    use time::OffsetDateTime;

    use super::*;
    use crate::test_log::with_captured_log;

    fn row(kid: &str, jwk: Value, active: bool) -> SigningKey {
        SigningKey {
            kid: kid.to_string(),
            public_jwk: jwk,
            active,
            expires_at: if active {
                None
            } else {
                Some(OffsetDateTime::now_utc() + time::Duration::hours(1))
            },
        }
    }

    fn jwk(kid: &str) -> Value {
        json!({
            "kty": "RSA",
            "n": format!("modulus-of-{kid}"),
            "e": "AQAB",
            "alg": "RS256",
            "use": "sig",
            "kid": kid,
        })
    }

    fn published_kids(document: &Value) -> Vec<String> {
        document
            .get("keys")
            .and_then(Value::as_array)
            .expect("the document has a `keys` array")
            .iter()
            .map(|key| {
                key.get("kid")
                    .and_then(Value::as_str)
                    .expect("every published key has a kid")
                    .to_string()
            })
            .collect()
    }

    /// The rotation window is the point of the whole table: the active key
    /// and a retired-but-unexpired key must *both* appear, or every token
    /// signed just before a rotation stops verifying.
    #[test]
    fn both_the_active_key_and_a_retired_but_unexpired_key_are_published() {
        let document = jwks_document(&[
            row("retired-but-in-window", jwk("retired-but-in-window"), false),
            row("active", jwk("active"), true),
        ]);

        assert_eq!(
            published_kids(&document),
            vec!["retired-but-in-window".to_string(), "active".to_string()],
            "both keys must be published, in the order the repository returned them"
        );
    }

    /// Input order is preserved (the repository orders by `created_at`), and
    /// the JWK is republished byte for byte — this document must not
    /// reformat, re-derive or "normalise" a key on the way out.
    #[test]
    fn the_document_preserves_input_order_and_republishes_each_jwk_verbatim() {
        let rows = [
            row("first", jwk("first"), false),
            row("second", jwk("second"), false),
            row("third", jwk("third"), true),
        ];
        let document = jwks_document(&rows);

        assert_eq!(
            published_kids(&document),
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
        assert_eq!(
            document.get("keys").and_then(Value::as_array),
            Some(
                &rows
                    .iter()
                    .map(|r| r.public_jwk.clone())
                    .collect::<Vec<_>>()
            ),
            "each stored JWK is published exactly as stored"
        );
    }

    /// A JWK with no `kid` is unusable by the validator
    /// (`require_kid(true)`), so it is dropped — and the drop is logged with
    /// the row's own key column so an operator can find it.
    #[test]
    fn a_jwk_without_a_kid_is_skipped_and_warned_about() {
        let ((), logs) = with_captured_log(|| {
            let document = jwks_document(&[
                row("good", jwk("good"), true),
                row(
                    "row-key-with-kidless-jwk",
                    json!({ "kty": "RSA", "n": "modulus", "e": "AQAB" }),
                    false,
                ),
            ]);
            assert_eq!(published_kids(&document), vec!["good".to_string()]);
        });

        assert!(
            logs.contains("row-key-with-kidless-jwk"),
            "the skipped row must be findable from the log: {logs}"
        );
        assert!(logs.contains("no `kid` member"), "{logs}");
    }

    /// A `public_jwk` column holding something that is not a JSON object
    /// (a string, a number, `null`) is the same class of problem and takes
    /// the same path, rather than reaching a verifier as a malformed entry.
    #[test]
    fn a_public_jwk_that_is_not_an_object_is_skipped_and_warned_about() {
        let ((), logs) = with_captured_log(|| {
            let document = jwks_document(&[row("not-an-object", json!("just a string"), true)]);
            assert!(
                published_kids(&document).is_empty(),
                "nothing publishable means an empty key set, not a malformed entry"
            );
        });

        assert!(logs.contains("not a JSON object"), "{logs}");
    }

    /// The document must stay a well-formed JWKS with an empty array when
    /// there is nothing to publish — never a missing `keys` member, and
    /// never `null`.
    #[test]
    fn no_keys_yields_an_empty_but_well_formed_key_set() {
        assert_eq!(jwks_document(&[]), json!({ "keys": [] }));
    }

    /// A row whose stored JWK names a different `kid` than its key column is
    /// still published — a verifier matches the JWK, not the column — but it
    /// is a sign the table was written by something other than
    /// `ensure_active_in_database`, so it must not pass silently.
    #[test]
    fn a_kid_that_disagrees_with_its_row_is_published_but_warned_about() {
        let ((), logs) = with_captured_log(|| {
            let document = jwks_document(&[row("column-says-this", jwk("jwk-says-that"), true)]);
            assert_eq!(published_kids(&document), vec!["jwk-says-that".to_string()]);
        });

        assert!(logs.contains("column-says-this"), "{logs}");
        assert!(logs.contains("disagrees"), "{logs}");
    }

    /// The cache header is part of the contract for every intermediary that
    /// caches this document — see [`JWKS_CACHE_MAX_AGE`]. Asserted on a real
    /// rendered response, not on the constant.
    #[test]
    fn the_response_is_publicly_cacheable_for_the_documented_window() {
        // `cache_control()` is the handler's own construction, not a copy of
        // it — changing the constant or the directive fails here.
        let response = (
            cache_control(),
            axum::Json(jwks_document(&[row("active", jwk("active"), true)])),
        )
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("public, max-age=300"),
            "a public key set is cacheable by anyone, for five minutes"
        );
    }
}
