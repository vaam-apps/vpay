//! The `/v1` operations a signed request can bind, vendored from
//! `vpay-wire` (VP-5).
//!
//! A signed request binds the **digest of the operation it calls**
//! (`Cratestack-Contract`, cratestack binding version 2): the server finds that
//! digest in `vpay_wire::V1_CONTRACTS` and refuses anything it does not hold.
//! This crate is what a merchant depends on, and it must stay publishable
//! without the vpay workspace, so it does not depend on `vpay-wire`. It carries
//! its own copy of the table instead, and `backends/crates/vpay-api`'s
//! `sdk_contracts` test (in the workspace, where both crates are visible) fails
//! when the two disagree — in the table, in the digest derivation, or in the
//! way a concrete path is matched to a row. A copy that the build checks is a
//! vendored file; a copy nothing checks is a second source of truth.
//!
//! The module is `#[doc(hidden)] pub` for exactly that test (and for this
//! crate's own wire-level fixtures); it is not part of the API a merchant calls.
//!
//! # What the digest is
//!
//! `SHA-256(DOMAIN || "rest" 0x00 || METHOD 0x00 || template)`: the operation's
//! identity, not a hash of its body shape (`vpay_wire`'s crate docs say what that
//! does and does not cover). `HEAD` is never listed; no resource method sends one.

use std::fmt;

use sha2::{Digest, Sha256};

/// No `/v1` operation matches a method and path.
///
/// Its own type, and not a variant of the crate's `Error`, so this file can be
/// compiled without the rest of the crate: `sdks/rust/wasm` includes it by
/// path, and the sealer it builds must bind exactly the table this one holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoOperation {
    /// The method that was asked.
    pub method: String,
    /// The path that was asked, as given.
    pub path: String,
}

impl fmt::Display for NoOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no /v1 operation matches {} {}", self.method, self.path)
    }
}

impl std::error::Error for NoOperation {}

/// Domain-separation tag of [`contract_digest`]. Byte for byte
/// `vpay_wire::CONTRACT_DOMAIN`.
pub const CONTRACT_DOMAIN: &[u8] = b"vpay/v1-route-contract/v1\0";

/// Every operation under `/v1`: `(METHOD, route template)`, the template
/// relative to the `/v1` mount and spelled as the server's router spells it.
/// Same rows, same order as `vpay_wire::V1_OPERATIONS`.
pub const OPERATIONS: &[(&str, &str)] = &[
    ("POST", "/payment_intents"),
    ("GET", "/payment_intents"),
    ("GET", "/payment_intents/{id}"),
    ("POST", "/payment_intents/{id}/confirm"),
    ("POST", "/payment_intents/{id}/cancel"),
    ("GET", "/events"),
    ("GET", "/account_holders"),
    ("GET", "/events/{id}"),
    ("POST", "/checkout/sessions"),
    ("GET", "/checkout/sessions"),
    ("GET", "/checkout/sessions/{id}"),
    ("POST", "/checkout/sessions/{id}/expire"),
    ("POST", "/refunds"),
    ("GET", "/refunds"),
    ("GET", "/refunds/{id}"),
    ("POST", "/refunds/{id}"),
    ("POST", "/refunds/{id}/cancel"),
    ("POST", "/customers"),
    ("GET", "/customers"),
    ("GET", "/customers/{id}"),
    ("POST", "/customers/{id}"),
    ("DELETE", "/customers/{id}"),
    ("POST", "/invoices"),
    ("GET", "/invoices"),
    ("GET", "/invoices/{id}"),
    ("POST", "/invoices/{id}"),
    ("PATCH", "/invoices/{id}"),
    ("DELETE", "/invoices/{id}"),
    ("POST", "/invoices/{id}/finalize"),
    ("POST", "/invoices/{id}/void"),
    ("POST", "/invoices/{id}/mark_uncollectible"),
    ("POST", "/invoices/{id}/pay"),
    ("POST", "/invoice_items"),
    ("GET", "/invoice_items/{id}"),
    ("POST", "/invoice_items/{id}"),
    ("PATCH", "/invoice_items/{id}"),
    ("DELETE", "/invoice_items/{id}"),
];

/// The 32 bytes a call to `method template` binds. Byte for byte
/// `vpay_wire::contract_digest`.
///
/// ```
/// let digest = vpay_sdk::contracts::contract_digest("GET", "/events");
/// assert_eq!(digest, vpay_sdk::contracts::contract_digest("GET", "/events"));
/// assert_ne!(digest, vpay_sdk::contracts::contract_digest("POST", "/events"));
/// ```
#[must_use]
pub fn contract_digest(method: &str, template: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CONTRACT_DOMAIN);
    hasher.update(b"rest\0");
    hasher.update(method.as_bytes());
    hasher.update(b"\0");
    hasher.update(template.as_bytes());
    hasher.finalize().into()
}

/// A concrete request path matched to its operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    /// The route template the signature binds, e.g. `/payment_intents/{id}`.
    pub template: &'static str,
    /// The path parameter values, **percent-decoded**, in template order —
    /// what the server's router hands the layer (axum decodes), and so what the
    /// signature must bind.
    pub params: Vec<String>,
}

/// Matches `method` and a concrete path (relative to `/v1`, as the resource
/// methods build it, ids already percent-encoded) to the operation it calls.
///
/// # Errors
///
/// [`NoOperation`] when no operation matches. A
/// path that matches nothing is a bug in this crate or a method this crate does
/// not have, and it is refused before anything is signed: a signature over a
/// route the server will not bind is a `401` with no hint at the cause.
///
/// ```
/// use vpay_sdk::contracts::operation_for;
///
/// let bound = operation_for("POST", "/payment_intents/pi_1/confirm").unwrap();
/// assert_eq!(bound.template, "/payment_intents/{id}/confirm");
/// assert_eq!(bound.params, ["pi_1"]);
///
/// // The server binds the DECODED value.
/// assert_eq!(operation_for("GET", "/customers/cus%20x").unwrap().params, ["cus x"]);
/// assert!(operation_for("GET", "/balance").is_err());
/// ```
pub fn operation_for(method: &str, path: &str) -> Result<Bound, NoOperation> {
    let no_match = || NoOperation {
        method: method.to_owned(),
        path: path.to_owned(),
    };
    let segments: Vec<&str> = path
        .strip_prefix('/')
        .ok_or_else(no_match)?
        .split('/')
        .collect();
    'rows: for (row_method, template) in OPERATIONS {
        if *row_method != method {
            continue;
        }
        let pattern: Vec<&str> = template.trim_start_matches('/').split('/').collect();
        if pattern.len() != segments.len() {
            continue;
        }
        let mut params = Vec::new();
        for (wanted, got) in pattern.iter().zip(&segments) {
            if wanted.starts_with('{') {
                if got.is_empty() {
                    continue 'rows;
                }
                params.push(percent_decode(got).ok_or_else(no_match)?);
            } else if wanted != got {
                continue 'rows;
            }
        }
        return Ok(Bound { template, params });
    }
    Err(no_match())
}

/// Decodes `%XX` escapes; `None` for a malformed escape or a result that is
/// not UTF-8 (axum refuses the same).
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        if byte == b'%' {
            let hex = input.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_derivation_is_frozen_at_the_values_vpay_wire_freezes() {
        // The same two vectors as `vpay_wire`'s `the_derivation_is_frozen`.
        let hex = |digest: [u8; 32]| {
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            hex(contract_digest("GET", "/events")),
            "d6bc1e04aa113bb33de96d31adac148887736d3c729bd0d04e43793dd4dce459"
        );
        assert_eq!(
            hex(contract_digest("POST", "/payment_intents/{id}/confirm")),
            "6cc558de7a936140f96f007b2ea9508fd06fc673bf2781cf99bbd1692ddf3ae1"
        );
    }

    #[test]
    fn every_operation_matches_itself_with_one_parameter_per_placeholder() {
        for (method, template) in OPERATIONS {
            let concrete = template.replace("{id}", "x_1");
            let bound = operation_for(method, &concrete).expect("a row matches itself");
            assert_eq!(bound.template, *template, "{method} {concrete}");
            assert_eq!(
                bound.params.len(),
                template.matches('{').count(),
                "{method} {template}"
            );
        }
    }

    #[test]
    fn a_path_is_never_ambiguous_across_rows_of_one_method() {
        // Each concrete path built from a row must match exactly that row: the
        // first-match loop above is only sound if no two rows overlap.
        for (method, template) in OPERATIONS {
            let concrete = template.replace("{id}", "x_1");
            let matches = OPERATIONS
                .iter()
                .filter(|(m, t)| {
                    m == method && operation_for(method, &concrete).is_ok_and(|b| b.template == *t)
                })
                .count();
            assert_eq!(matches, 1, "{method} {concrete}");
        }
    }

    #[test]
    fn what_is_not_an_operation_is_refused_before_signing() {
        for (method, path) in [
            ("GET", "/balance"),
            ("PUT", "/customers/cus_1"),
            ("GET", "customers"),
            ("GET", "/customers/"),
            ("GET", "/customers/cus_1/extra"),
            ("GET", "/customers/%zz"),
            ("DELETE", "/payment_intents/pi_1"),
        ] {
            assert!(operation_for(method, path).is_err(), "{method} {path}");
        }
    }
}
