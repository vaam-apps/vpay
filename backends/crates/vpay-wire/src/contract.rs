//! The digest an operation binds, and the table of them.

use std::sync::LazyLock;

use cratestack_core::AcceptedContracts;
use sha2::{Digest, Sha256};

use crate::operations::{DASH_OPERATIONS, Operation, SVC_OPERATIONS, V1_OPERATIONS};

/// Domain-separation tag of [`contract_digest`], in the shape cratestack
/// gives its own (`cratestack/op-contract/v1\0`). If the derivation changes,
/// the tag moves to `v2`: two derivations must never produce the same bytes
/// for different inputs.
pub const CONTRACT_DOMAIN: &[u8] = b"vpay/v1-route-contract/v1\0";

/// [`CONTRACT_DOMAIN`] for `/dash/v1` (VP-6a). A different tag, not the same
/// one over a different path: `GET /payment_intents` is an operation on `/v1`
/// and another on `/dash/v1`, and a message signed for one must not be
/// accepted as the other even by a layer that was handed the wrong key set.
pub const DASH_CONTRACT_DOMAIN: &[u8] = b"vpay/dash-route-contract/v1\0";

/// [`CONTRACT_DOMAIN`] for `/svc/v1` (VP-6a), for [`DASH_CONTRACT_DOMAIN`]'s
/// reason.
pub const SVC_CONTRACT_DOMAIN: &[u8] = b"vpay/svc-route-contract/v1\0";

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
    digest_under(CONTRACT_DOMAIN, method, path)
}

/// [`contract_digest`] for an operation under `/dash/v1`.
///
/// ```
/// assert_ne!(
///     vpay_wire::dash_contract_digest("GET", "/payment_intents"),
///     vpay_wire::contract_digest("GET", "/payment_intents"),
/// );
/// ```
#[must_use]
pub fn dash_contract_digest(method: &str, path: &str) -> [u8; 32] {
    digest_under(DASH_CONTRACT_DOMAIN, method, path)
}

/// [`contract_digest`] for an operation under `/svc/v1`.
///
/// ```
/// assert_ne!(
///     vpay_wire::svc_contract_digest("POST", "/console_keys"),
///     vpay_wire::dash_contract_digest("POST", "/console_keys"),
/// );
/// ```
#[must_use]
pub fn svc_contract_digest(method: &str, path: &str) -> [u8; 32] {
    digest_under(SVC_CONTRACT_DOMAIN, method, path)
}

fn digest_under(domain: &[u8], method: &str, path: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(b"rest\0");
    hasher.update(method.as_bytes());
    hasher.update(b"\0");
    hasher.update(path.as_bytes());
    hasher.finalize().into()
}

/// One leaked row per operation, each with the one digest `digest` derives.
fn table(
    operations: &'static [Operation],
    digest: fn(&str, &str) -> [u8; 32],
) -> AcceptedContracts {
    let rows: Vec<(&'static str, &'static [[u8; 32]])> = operations
        .iter()
        .map(|operation| {
            let key: &'static str =
                Box::leak(operation_key(operation.method, operation.path).into_boxed_str());
            let digests: &'static [[u8; 32]] =
                Box::leak(Box::new([digest(operation.method, operation.path)]));
            (key, digests)
        })
        .collect();
    Box::leak(rows.into_boxed_slice())
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
pub static V1_CONTRACTS: LazyLock<AcceptedContracts> =
    LazyLock::new(|| table(V1_OPERATIONS, contract_digest));

/// What the `/dash/v1` `EnvelopeLayer` accepts for the hand-written reads:
/// one digest per [`DASH_OPERATIONS`] row, under [`DASH_CONTRACT_DOMAIN`].
pub static DASH_CONTRACTS: LazyLock<AcceptedContracts> =
    LazyLock::new(|| table(DASH_OPERATIONS, dash_contract_digest));

/// What the `/svc/v1` `EnvelopeLayer` accepts: one digest per
/// [`SVC_OPERATIONS`] row, under [`SVC_CONTRACT_DOMAIN`].
pub static SVC_CONTRACTS: LazyLock<AcceptedContracts> =
    LazyLock::new(|| table(SVC_OPERATIONS, svc_contract_digest));

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

    /// `/dash/v1` and `/svc/v1` bind under their own domain tags (VP-6a): the
    /// same method and path is a different operation on each surface, and the
    /// derivations are frozen like `/v1`'s so a refactor cannot move them.
    #[test]
    fn the_three_surfaces_never_share_a_digest_and_the_new_two_are_frozen() {
        let (v1, dash, svc) = (
            contract_digest("GET", "/payment_intents"),
            dash_contract_digest("GET", "/payment_intents"),
            svc_contract_digest("GET", "/payment_intents"),
        );
        assert!(v1 != dash && dash != svc && v1 != svc);
        assert_eq!(
            hex(&dash),
            "8729fd35a8ec30362c77e1a780ed387271d15bb79d8b4b97e59dfd7ab804d0f8",
            "frozen: a change here refuses every console already built"
        );
        assert_eq!(
            hex(&svc_contract_digest("POST", "/console_keys")),
            "097c77e8078fff0499367d0007e48fad7ca3c75fac7a30f79cc6ba52f3c24b4a",
            "frozen: a change here refuses vaam's pairing calls"
        );
    }

    #[test]
    fn the_dash_and_svc_tables_have_one_row_and_one_digest_per_operation() {
        use crate::operations::{DASH_OPERATIONS, SVC_OPERATIONS};
        assert_eq!(DASH_CONTRACTS.len(), DASH_OPERATIONS.len());
        assert_eq!(SVC_CONTRACTS.len(), SVC_OPERATIONS.len());
        for operation in DASH_OPERATIONS {
            let digests = find_contract(&DASH_CONTRACTS, operation.method, operation.path)
                .unwrap_or_else(|| panic!("no row for {} {}", operation.method, operation.path));
            assert_eq!(
                digests,
                &[dash_contract_digest(operation.method, operation.path)]
            );
        }
        for operation in SVC_OPERATIONS {
            let digests = find_contract(&SVC_CONTRACTS, operation.method, operation.path)
                .unwrap_or_else(|| panic!("no row for {} {}", operation.method, operation.path));
            assert_eq!(
                digests,
                &[svc_contract_digest(operation.method, operation.path)]
            );
        }
        // The other surface's operations are not rows here.
        assert!(find_contract(&DASH_CONTRACTS, "POST", "/console_keys").is_none());
        assert!(find_contract(&SVC_CONTRACTS, "GET", "/payment_intents").is_none());
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
