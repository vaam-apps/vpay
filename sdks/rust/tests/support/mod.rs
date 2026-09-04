//! Shared fixtures for `vpay-sdk`'s integration tests.
//!
//! Two things live here rather than being repeated per test binary: real RSA
//! keypair generation (the slow part of this suite — one keypair per test
//! would dominate its runtime) and the small amount of `wiremock` plumbing
//! every HTTP test needs.
//!
//! Nothing in this module is reachable from the shipping crate: `tests/` is
//! compiled only for `cargo test`/`cargo nextest`, and
//! `cargo xtask verify-no-mocks` is what enforces that a stub never reaches
//! `vpay-server`/`vpay-worker-bin` (AGENTS.md, rule 1). The SDK is not one of
//! those binaries, but the rule's reasoning still applies: no `#[cfg]`-
//! selected fake exists inside `src/`.

// `clippy.toml` exempts `#[test]` functions from the workspace `unwrap`/
// `expect`/`panic` deny, but not helper functions in a support module that
// merely *supports* them, and `indexing_slicing` has no test exemption at
// all. Same allow list, for the same reason, as
// `backends/apps/vpay-server/tests/cli.rs`. Each helper still fails loudly:
// a panic here is a broken fixture, which is exactly how a test should
// report one.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
use rsa::traits::PublicKeyParts as _;
use serde_json::{Value, json};

/// An RSA keypair in the two shapes these tests need it: the private half as
/// a PEM (what a merchant hands [`vpay_sdk::Credentials::rsa_pem`]) and the
/// public half as a JWK (what vpay would hold in its YAML registration).
///
/// Real generated key material, never a hard-coded pair: a fixture keypair
/// shared with the *verifier* under test would let a broken signature path
/// still "verify", because both sides would be reading the same canned
/// artefact.
pub(crate) struct TestKey {
    /// The `kid` stamped onto the JWK, if this key is one of several.
    pub(crate) kid: Option<String>,
    /// PKCS#1 PEM of the private half.
    pub(crate) pem: String,
    /// The public half as a JWK object (not a set).
    pub(crate) jwk: Value,
}

/// Generates a 2048-bit RSA keypair and derives its public JWK.
///
/// 2048 rather than 3072/4096 purely for suite runtime; the property under
/// test is that the OP verifier accepts a signature this SDK produced, and
/// that is independent of modulus size.
pub(crate) fn generate_key(kid: Option<&str>) -> TestKey {
    // `OsRng` (not a seeded/deterministic RNG), matching
    // `vpay_api::resource_auth`'s own test keypair helper.
    let mut rng = rand::rngs::OsRng;
    let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa key generation succeeds");
    let public_key = private_key.to_public_key();

    let pem = private_key
        .to_pkcs1_pem(LineEnding::LF)
        .expect("pkcs1 pem encoding succeeds")
        .to_string();

    let mut jwk = json!({
        "kty": "RSA",
        "use": "sig",
        "alg": "RS256",
        "n": URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be()),
        "e": URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be()),
    });
    if let Some(kid) = kid {
        jwk["kid"] = json!(kid);
    }

    TestKey {
        kid: kid.map(str::to_string),
        pem,
        jwk,
    }
}

/// Wraps JWKs into the `{"keys": [...]}` set shape
/// `authkestra_op::client_assertion::select_key` expects.
pub(crate) fn jwks(keys: &[&TestKey]) -> Value {
    json!({ "keys": keys.iter().map(|k| k.jwk.clone()).collect::<Vec<_>>() })
}

/// The `application/x-www-form-urlencoded` pairs of a recorded request body,
/// in wire order.
///
/// Deliberately does *not* use a URL-decoding parser: these tests assert on
/// the bytes the SDK put on the wire (the form contract is byte-level — see
/// `sdks/rust/src/form.rs`), so decoding first would hide exactly the kind of
/// escaping drift the encoder tests exist to catch.
pub(crate) fn form_pairs(body: &[u8]) -> Vec<(String, String)> {
    let body = std::str::from_utf8(body).expect("request body is UTF-8");
    if body.is_empty() {
        return Vec::new();
    }
    body.split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (pair.to_string(), String::new()),
        })
        .collect()
}

/// Looks a field out of [`form_pairs`]' output.
pub(crate) fn form_field<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Percent-decodes a form value from [`form_pairs`].
///
/// [`form_pairs`] deliberately hands back the raw wire bytes; a test that
/// wants the *value* (a client assertion, say) decodes it here rather than
/// assuming the encoder happened to leave it alone.
pub(crate) fn percent_decode(input: &str) -> String {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes.get(i) {
            Some(b'%') => {
                let hex = input.get(i + 1..i + 3).expect("a %XX escape is complete");
                out.push(u8::from_str_radix(hex, 16).expect("a %XX escape is hex"));
                i += 3;
            }
            Some(b) => {
                out.push(*b);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8(out).expect("a decoded form value is UTF-8")
}

/// A `TokenResponse` body shaped exactly like
/// `authkestra_op::handlers::token::TokenResponse`
/// (`docs/flows/merchant-auth.md`, "Success response").
pub(crate) fn token_response(access_token: &str, expires_in: u64) -> Value {
    json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": expires_in,
    })
}

/// A `payment_intent` object with every field the wire contract lists.
pub(crate) fn payment_intent_json(id: &str) -> Value {
    json!({
        "id": id,
        "object": "payment_intent",
        "amount": 5000,
        "currency": "xaf",
        "status": "requires_payment_method",
        "payment_method_types": ["mtn_momo"],
        "next_action": null,
        "last_payment_error": null,
        "metadata": { "order_id": "1234" },
        "description": null,
        "created": 1_753_401_600,
        "livemode": false,
    })
}

/// A `checkout.session` object with every field the wire contract lists —
/// the hosted shape, whose `url` carries the session secret in its fragment
/// (Step 9's D6).
pub(crate) fn checkout_session_json(id: &str, client_secret: Option<&str>) -> Value {
    let mut object = json!({
        "id": id,
        "object": "checkout.session",
        "livemode": false,
        "payment_intent": "pi_123",
        "ui_mode": "hosted",
        "status": "open",
        "payment_status": "unpaid",
        "success_url": "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
        "cancel_url": "https://shop.example/cancel",
        "return_url": null,
        "url": format!("https://checkout.example/c/{id}#{id}_secret_abc123"),
        "expires_at": 1_700_086_400,
        "created": 1_700_000_000,
    });
    if let Some(secret) = client_secret {
        object["client_secret"] = json!(secret);
    }
    object
}
