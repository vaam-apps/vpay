//! The digest an operation binds, and the table of them.

use std::sync::LazyLock;

use cratestack_core::AcceptedContracts;
use sha2::{Digest, Sha256};

use crate::operations::V1_OPERATIONS;

/// Domain-separation tag of [`contract_digest`], in the shape cratestack
/// gives its own (`cratestack/op-contract/v1\0`). If the derivation changes,
/// the tag moves to `v2`: two derivations must never produce the same bytes
/// for different inputs.
pub const CONTRACT_DOMAIN: &[u8] = b"vpay/v1-route-contract/v1\0";

/// The row key of an operation: `"<METHOD> <path>"`, as cratestack keys a
/// REST op (`GET /payment_intents/{id}`), path relative to the mount.
///
/// ```
/// assert_eq!(vpay_wire::operation_key("GET", "/events/{id}"), "GET /events/{id}");
/// ```
#[must_use]
pub fn operation_key(method: &str, path: &str) -> String {
    format!("{method} {path}")
}

/// The 32 bytes a call to `method path` binds.
///
/// `SHA-256(CONTRACT_DOMAIN || "rest" 0x00 || method 0x00 || path)`. A NUL
/// cannot occur in a method or a route template, so the fields are
/// unambiguous without a length prefix. See the crate docs for what this does
/// and does not cover: it is the operation's identity, not a hash of its
/// body's shape.
///
/// ```
/// let get = vpay_wire::contract_digest("GET", "/events");
/// assert_eq!(get, vpay_wire::contract_digest("GET", "/events"));
/// // The method is part of the identity: the same path with another verb is
/// // another operation.
/// assert_ne!(get, vpay_wire::contract_digest("POST", "/events"));
/// ```
#[must_use]
pub fn contract_digest(method: &str, path: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CONTRACT_DOMAIN);
    hasher.update(b"rest\0");
    hasher.update(method.as_bytes());
    hasher.update(b"\0");
    hasher.update(path.as_bytes());
    hasher.finalize().into()
}

/// What the `/v1` `EnvelopeLayer` accepts: one digest per [`V1_OPERATIONS`]
/// row, keyed by [`operation_key`].
///
/// A `LazyLock` over a leaked table and not a `const`, because
/// `AcceptedContracts` is `&'static [(&'static str, &'static [[u8; 32]])]` and
/// SHA-256 is not const-evaluable; the alternative was thirty-seven opaque hex
/// literals pasted from a script, which would be a second copy of what
/// [`contract_digest`] computes. The table is built once on first use and lives
/// for the process (about thirty-seven small allocations, never freed on
/// purpose). Dereference it where the layer wants the slice:
/// `*V1_CONTRACTS`.
///
/// Each row has exactly one digest. cratestack lets a server keep older,
/// still-compatible digests after a change (`contracts =` / `ContractLock`);
/// nothing here has a previous contract to keep.
pub static V1_CONTRACTS: LazyLock<AcceptedContracts> = LazyLock::new(|| {
    let rows: Vec<(&'static str, &'static [[u8; 32]])> = V1_OPERATIONS
        .iter()
        .map(|operation| {
            let key: &'static str =
                Box::leak(operation_key(operation.method, operation.path).into_boxed_str());
            let digests: &'static [[u8; 32]] = Box::leak(Box::new([contract_digest(
                operation.method,
                operation.path,
            )]));
            (key, digests)
        })
        .collect();
    Box::leak(rows.into_boxed_slice())
});

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use cratestack_core::find_contract;

    use super::*;

    fn hex(digest: &[u8; 32]) -> String {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The derivation, frozen. A client built before a change to
    /// [`contract_digest`] binds the old bytes, and the server answers every
    /// one of its requests `426 contract_unsupported`; so a change to the
    /// derivation has to fail here and be made on purpose (and move the
    /// domain tag to `v2`), never be a side effect of a refactor.
    #[test]
    fn the_derivation_is_frozen() {
        assert_eq!(
            hex(&contract_digest("GET", "/events")),
            "d6bc1e04aa113bb33de96d31adac148887736d3c729bd0d04e43793dd4dce459"
        );
        assert_eq!(
            hex(&contract_digest("POST", "/payment_intents/{id}/confirm")),
            "6cc558de7a936140f96f007b2ea9508fd06fc673bf2781cf99bbd1692ddf3ae1"
        );
    }

    #[test]
    fn every_operation_has_exactly_one_row_and_one_digest() {
        assert_eq!(V1_CONTRACTS.len(), V1_OPERATIONS.len());
        for operation in V1_OPERATIONS {
            let digests = find_contract(&V1_CONTRACTS, operation.method, operation.path)
                .unwrap_or_else(|| panic!("no row for {} {}", operation.method, operation.path));
            assert_eq!(
                digests,
                &[contract_digest(operation.method, operation.path)]
            );
        }
    }

    #[test]
    fn no_two_operations_share_a_key_or_a_digest() {
        let keys: HashSet<&str> = V1_CONTRACTS.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys.len(), V1_CONTRACTS.len(), "a duplicate key");
        let digests: HashSet<[u8; 32]> = V1_CONTRACTS
            .iter()
            .flat_map(|(_, digests)| digests.iter().copied())
            .collect();
        assert_eq!(digests.len(), V1_CONTRACTS.len(), "a duplicate digest");
    }

    #[test]
    fn a_head_resolves_to_its_get_and_an_unrouted_method_to_nothing() {
        assert_eq!(
            find_contract(&V1_CONTRACTS, "HEAD", "/events/{id}"),
            find_contract(&V1_CONTRACTS, "GET", "/events/{id}")
        );
        assert!(find_contract(&V1_CONTRACTS, "HEAD", "/events/{id}").is_some());
        assert!(find_contract(&V1_CONTRACTS, "PUT", "/events/{id}").is_none());
        assert!(find_contract(&V1_CONTRACTS, "GET", "/nonexistent").is_none());
    }

    #[test]
    fn operations_are_well_formed() {
        for operation in V1_OPERATIONS {
            assert!(
                matches!(
                    operation.method,
                    "GET" | "POST" | "PATCH" | "PUT" | "DELETE"
                ),
                "{operation:?}: methods are upper-case and HEAD is never listed"
            );
            assert!(operation.path.starts_with('/'), "{operation:?}");
            assert!(!operation.path.starts_with("/v1"), "{operation:?}");
            assert!(!operation.path.ends_with('/'), "{operation:?}");
            assert!(
                !operation.path.contains('\0') && !operation.method.contains('\0'),
                "{operation:?}"
            );
        }
        let distinct: HashSet<_> = V1_OPERATIONS.iter().collect();
        assert_eq!(distinct.len(), V1_OPERATIONS.len());
    }
}
