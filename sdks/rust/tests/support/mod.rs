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
        // `customer` since 2026-09-06 (S4a). Present and null, like every
        // other nullable key: the server emits every documented key, and a
        // fixture that omitted it would stop this file being the wire shape.
        "customer": null,
        "created": 1_753_401_600,
        "livemode": false,
    })
}

/// A `customer` object with every field the wire contract lists (S4a).
///
/// **Phone-only**, deliberately: it is the maintainer's decision of
/// 2026-09-05 made visible in the fixture every customer test decodes, so a
/// change that made `name` or `email` required would fail here rather than in
/// a container.
pub(crate) fn customer_json(id: &str) -> Value {
    json!({
        "id": id,
        "object": "customer",
        "name": null,
        "email": null,
        // Canonical, as the server stores and renders it — not the `+237 6 …`
        // a merchant would have typed.
        "phone": "237600000200",
        // The server renders one nested object with all EIGHT components,
        // nulls included, or `null` for a customer with no address at all.
        // This fixture carries the object, so a decode that dropped the key
        // or flattened it fails rather than reading as "no address".
        //
        // `latitude_microdeg` / `longitude_microdeg` are vpay's own — Stripe's
        // address has no coordinate — and they are whole microdegrees, never
        // degrees. Written as bare JSON integers here on purpose: if `Address`
        // typed them as a float this fixture would still decode, so
        // `a_customer_decodes_its_coordinate_as_whole_microdegrees` asserts
        // the value rather than only the decode.
        "address": {
            "line1": "12 Rue Njo-Njo",
            "line2": null,
            "city": "Douala",
            "state": null,
            "postal_code": null,
            "country": "CM",
            "latitude_microdeg": 4_061_000,
            "longitude_microdeg": 9_786_000,
        },
        "metadata": { "order_id": "1234" },
        "created": 1_753_401_600,
        "livemode": false,
    })
}

/// The customer object as it comes back **after an erasure**: every
/// identifier the redaction marker, `deleted: true`, and the merchant's own
/// `metadata` untouched.
///
/// A fixture of its own rather than a flag on [`customer_json`], because it
/// is a different shape — ten keys rather than nine — and the test that reads
/// it is about the key that is only there sometimes.
///
/// The address is the marker in the six formal components and **`null`** in
/// the two coordinate ones, which is what the server really sends: a
/// coordinate is an integer and there is no integer that is not a possible
/// place, so the marker is not a value it can take. A fixture that put the
/// marker there instead would be asserting a body the server cannot produce
/// — and, with `Option<i64>`, would not even decode.
pub(crate) fn erased_customer_json(id: &str) -> Value {
    json!({
        "id": id,
        "object": "customer",
        "name": "[redacted]",
        "email": "[redacted]",
        "phone": "[redacted]",
        "address": {
            "line1": "[redacted]",
            "line2": "[redacted]",
            "city": "[redacted]",
            "state": "[redacted]",
            "postal_code": "[redacted]",
            "country": "[redacted]",
            "latitude_microdeg": null,
            "longitude_microdeg": null,
        },
        "metadata": { "order_id": "1234" },
        "created": 1_753_401_600,
        "livemode": false,
        "deleted": true,
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
        "customer": null,
        "expires_at": 1_700_086_400,
        "created": 1_700_000_000,
    });
    if let Some(secret) = client_secret {
        object["client_secret"] = json!(secret);
    }
    object
}

/// An `invoice` object with every field the wire contract lists (S4b) — all
/// **nineteen** keys, one line expanded.
///
/// The count is the point: this object is the `data.object` of all four
/// `invoice.*` event types, so a twentieth key is signed, delivered and
/// stored forever. `vpay_api`'s
/// `the_invoice_object_is_the_documented_nineteen_keys` holds the number on
/// the server side; this fixture is what the SDK decodes, so a key that
/// appeared on one side and not the other shows up as a decode difference
/// here rather than in a container.
///
/// `status` is `open` and `number` is assigned, deliberately: a draft fixture
/// would let `number: Option<String>` be `None` in every case and nothing
/// would ever decode the assigned form.
pub(crate) fn invoice_json(id: &str) -> Value {
    json!({
        "id": id,
        "object": "invoice",
        // Never `null`, unlike a payment intent's.
        "customer": "cus_1",
        "currency": "xaf",
        "status": "open",
        "number": "A7K3M9QP-000001",
        "amount_due": 11_000,
        "amount_paid": 0,
        "amount_remaining": 11_000,
        // Zero on every real invoice today (no rail can refund), and spelled
        // here anyway: `Invoice::amount_refunded` is `#[serde(default)]`, so a
        // fixture that omitted it would decode identically whether the server
        // sent the key or not.
        "amount_refunded": 0,
        "due_date": null,
        "description": "September hosting",
        "metadata": { "order_id": "1234" },
        "payment_intent": null,
        "hosted_invoice_url": null,
        "lines": {
            "object": "list",
            "has_more": false,
            // `/v1/invoice_items`, a route that exists — not Stripe's
            // `/v1/invoices/{id}/lines`, which vpay does not serve.
            "url": "/v1/invoice_items",
            "data": [invoice_line_json("ii_1")],
        },
        "status_transitions": {
            "finalized_at": 1_753_401_600_i64,
            "paid_at": null,
            "voided_at": null,
            "marked_uncollectible_at": null,
        },
        "created": 1_753_401_600_i64,
        "livemode": false,
    })
}

/// A `line_item` object with every field the wire contract lists (S4b).
///
/// Its `object` is `"line_item"` and the route that addresses it is
/// `/v1/invoice_items`; both spellings are the wire's, and the fixture is
/// where that stops being surprising.
pub(crate) fn invoice_line_json(id: &str) -> Value {
    json!({
        "id": id,
        "object": "line_item",
        "description": "Hosting",
        "quantity": 2,
        "unit_amount": 5_500,
        // `quantity * unit_amount`, computed by the database and never sent.
        "amount": 11_000,
        "currency": "xaf",
        "livemode": false,
    })
}
